---
layout: home

hero:
  name: sksync
  text: Sync Agent Skills across coding agents
  tagline: One config file. One source of truth. sksync resolves Agent Skills from GitHub, skills.sh, or local directories and symlinks them into every agent's skills directory — reproducibly, via a portable lockfile.
  actions:
    - theme: brand
      text: Get Started
      link: /quickstart
    - theme: alt
      text: Install
      link: /install
    - theme: alt
      text: View on GitHub
      link: https://github.com/takemo101/sksync

features:
  - icon: 🔗
    title: One config, many agents
    details: Declare a skill once with the agents that should see it. sksync symlinks the single skill body into Claude Code, Codex, Gemini, OpenCode, Pi, Antigravity, and 40+ other agent skills directories.
  - icon: 📦
    title: npm-like dependency model
    details: "`sksync add` / `remove` / `outdated` / `update` / `install` manage Agent Skills like package dependencies, recorded in `sksync.config.json`."
  - icon: 🌐
    title: Flexible sources
    details: Add skills from GitHub shorthand, tree URLs, skills.sh, or local directories. Repo-root sources auto-discover SKILL.md so you can point at a whole repo and pick.
  - icon: 🔒
    title: Reproducible via lockfile
    details: A portable lockfile v4 pins resolved commits and file hashes so `sksync install` reconstructs the exact same skills across macOS and Linux.
  - icon: 🛡️
    title: Safe by default
    details: Never overwrites plain files, only removes symlinks it manages, rolls back config on failed adds, and refuses to escape the project root for project-scope targets.
  - icon: 🧙
    title: CLI + interactive wizard
    details: Drive everything from the CLI, or run `sksync wizard` for a guided prompt flow to add, attach, detach, remove, and apply.
---
