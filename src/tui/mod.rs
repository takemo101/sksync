use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use inquire::{Confirm, MultiSelect, Select, Text};

use crate::application::config::ResolvedConfig;
use crate::application::ports::ConfigStore;
use crate::infrastructure::json::FileConfigStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Intent {
    AddSkill,
    RemoveSkill,
    RemoveAgent,
    Status,
    Apply,
    Quit,
}

impl fmt::Display for Intent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::AddSkill => "skill を追加する",
            Self::RemoveSkill => "skill を削除する",
            Self::RemoveAgent => "特定 agent から skill を外す",
            Self::Status => "状態を確認する",
            Self::Apply => "apply する",
            Self::Quit => "終了する",
        };
        formatter.write_str(label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigScope {
    Project,
    Global,
}

impl ConfigScope {
    fn is_global(self) -> bool {
        matches!(self, Self::Global)
    }
}

impl fmt::Display for ConfigScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Project => "project config (./sksync.config.json)",
            Self::Global => "global config (~/.config/sksync/config.json)",
        };
        formatter.write_str(label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoveMode {
    Normal,
    KeepFiles,
    ConfigOnly,
}

impl RemoveMode {
    fn append_args(self, args: &mut Vec<String>) {
        match self {
            Self::Normal => {}
            Self::KeepFiles => args.push("--keep-files".to_owned()),
            Self::ConfigOnly => args.push("--config-only".to_owned()),
        }
    }
}

impl fmt::Display for RemoveMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Normal => "通常削除（オプションなし・symlink も削除）",
            Self::KeepFiles => "skill 本体を残す（--keep-files）",
            Self::ConfigOnly => "config / lockfile だけ変更（--config-only）",
        };
        formatter.write_str(label)
    }
}

pub fn run(project_root: PathBuf) -> Result<()> {
    println!("sksync prompt TUI");
    println!("Project: {}", project_root.display());

    loop {
        let intent = Select::new(
            "何をしますか?",
            vec![
                Intent::AddSkill,
                Intent::RemoveSkill,
                Intent::RemoveAgent,
                Intent::Status,
                Intent::Apply,
                Intent::Quit,
            ],
        )
        .prompt()
        .context("failed to read TUI selection")?;

        match intent {
            Intent::AddSkill => run_add_flow(&project_root)?,
            Intent::RemoveSkill => run_remove_flow(&project_root)?,
            Intent::RemoveAgent => run_remove_agent_flow(&project_root)?,
            Intent::Status => run_status_flow(&project_root)?,
            Intent::Apply => run_apply_flow(&project_root)?,
            Intent::Quit => return Ok(()),
        }
    }
}

fn run_add_flow(project_root: &PathBuf) -> Result<()> {
    let source = prompt_required("skill source")?;
    let name = Text::new("name override")
        .with_help_message("optional; leave blank to infer from source")
        .prompt()
        .context("failed to read name override")?;
    let agents = prompt_agents()?;
    let global = prompt_config_scope("どの config に追加しますか?")?.is_global();

    let mut args = vec!["add".to_owned(), source];
    for agent in agents {
        args.push("--agent".to_owned());
        args.push(agent);
    }
    if !name.trim().is_empty() {
        args.push("--name".to_owned());
        args.push(name.trim().to_owned());
    }
    if global {
        args.push("--global".to_owned());
    }

    confirm_and_run(project_root, "実行しますか?", args)
}

fn run_remove_flow(project_root: &PathBuf) -> Result<()> {
    let scope = prompt_config_scope("どの config から削除しますか?")?;
    let config = load_config_for_scope(project_root, scope)?;
    let skill = prompt_skill_from_config(&config, "削除する skill を選択してください")?;
    let mode = prompt_remove_mode()?;

    let mut args = vec!["remove".to_owned(), skill];
    if scope.is_global() {
        args.push("--global".to_owned());
    }
    mode.append_args(&mut args);

    confirm_and_run(project_root, "削除を実行しますか?", args)
}

fn run_remove_agent_flow(project_root: &PathBuf) -> Result<()> {
    let scope = prompt_config_scope("どの config を対象にしますか?")?;
    let config = load_config_for_scope(project_root, scope)?;
    let skill = prompt_skill_from_config(&config, "agent から外す skill を選択してください")?;
    let agents = prompt_agents_for_skill(&config, &skill)?;

    let mut args = vec!["remove".to_owned(), skill];
    for agent in agents {
        args.push("--agent".to_owned());
        args.push(agent);
    }
    if scope.is_global() {
        args.push("--global".to_owned());
    }

    confirm_and_run(project_root, "指定 agent から外しますか?", args)
}

fn run_status_flow(project_root: &PathBuf) -> Result<()> {
    let global = prompt_config_scope("どの config を確認しますか?")?.is_global();
    let check = prompt_confirm("check も実行しますか?", true)?;

    let mut list_args = vec!["list".to_owned()];
    if global {
        list_args.push("--global".to_owned());
    }
    run_sksync(project_root, &list_args)?;

    if check {
        let mut check_args = vec!["check".to_owned()];
        if global {
            check_args.push("--global".to_owned());
        }
        run_sksync(project_root, &check_args)?;
    }
    Ok(())
}

fn run_apply_flow(project_root: &PathBuf) -> Result<()> {
    let global = prompt_config_scope("どの config を apply しますか?")?.is_global();
    let force = prompt_confirm("safe な managed link の置き換えを許可しますか?", false)?;

    let mut plan_args = vec!["plan".to_owned()];
    if global {
        plan_args.push("--global".to_owned());
    }
    println!("dry-run plan:");
    run_sksync(project_root, &plan_args)?;

    let mut apply_args = vec!["apply".to_owned()];
    if global {
        apply_args.push("--global".to_owned());
    }
    if force {
        apply_args.push("--force".to_owned());
    }

    confirm_and_run(project_root, "apply を実行しますか?", apply_args)
}

fn confirm_and_run(project_root: &PathBuf, question: &str, args: Vec<String>) -> Result<()> {
    println!("予定: sksync {}", args.join(" "));
    if prompt_confirm(question, false)? {
        run_sksync(project_root, &args)?;
    }
    Ok(())
}

fn prompt_config_scope(message: &str) -> Result<ConfigScope> {
    Select::new(message, vec![ConfigScope::Project, ConfigScope::Global])
        .prompt()
        .context("failed to read config scope")
}

fn prompt_remove_mode() -> Result<RemoveMode> {
    Select::new(
        "削除モードを選択してください",
        vec![
            RemoveMode::Normal,
            RemoveMode::KeepFiles,
            RemoveMode::ConfigOnly,
        ],
    )
    .with_help_message("通常削除は CLI の `sksync remove <skill>` と同じです")
    .prompt()
    .context("failed to read remove mode")
}

fn prompt_skill_from_config(config: &ResolvedConfig, message: &str) -> Result<String> {
    let skills = config
        .skills
        .iter()
        .map(|skill| skill.name.as_str().to_owned())
        .collect::<Vec<_>>();
    if skills.is_empty() {
        bail!("no skills are configured");
    }
    Select::new(message, skills)
        .prompt()
        .context("failed to read skill selection")
}

fn prompt_agents_for_skill(config: &ResolvedConfig, skill_name: &str) -> Result<Vec<String>> {
    let skill = config
        .skills
        .iter()
        .find(|skill| skill.name.as_str() == skill_name)
        .with_context(|| format!("configured skill not found: {skill_name}"))?;
    let agents = skill
        .agents
        .iter()
        .map(|agent| agent.as_str().to_owned())
        .collect::<Vec<_>>();
    if agents.is_empty() {
        bail!("skill {skill_name} has no configured agents");
    }
    MultiSelect::new("外す agent を選択してください", agents)
        .with_help_message("space で選択、enter で確定")
        .prompt()
        .context("failed to read configured agent selection")
}

fn load_config_for_scope(project_root: &Path, scope: ConfigScope) -> Result<ResolvedConfig> {
    let path = config_path_for_scope(project_root, scope)?;
    if !path.exists() {
        bail!("config not found: {}", path.display());
    }
    FileConfigStore::new(path)
        .load()
        .context("failed to load config")
}

fn config_path_for_scope(project_root: &Path, scope: ConfigScope) -> Result<PathBuf> {
    match scope {
        ConfigScope::Project => Ok(project_root.join("sksync.config.json")),
        ConfigScope::Global => dirs::config_dir()
            .map(|dir| dir.join("sksync/config.json"))
            .context("failed to determine global config directory"),
    }
}

fn prompt_agents() -> Result<Vec<String>> {
    let selected = MultiSelect::new(
        "agents を選択してください",
        vec!["pi", "claude-code", "codex", "gemini", "opencode", "custom"],
    )
    .with_help_message("space で選択、enter で確定")
    .prompt()
    .context("failed to read agent selection")?;

    let mut agents = Vec::new();
    for agent in selected {
        if agent == "custom" {
            agents.extend(prompt_custom_agents()?);
        } else {
            agents.push(agent.to_owned());
        }
    }

    if agents.is_empty() {
        bail!("at least one agent is required");
    }
    agents.sort();
    agents.dedup();
    Ok(agents)
}

fn prompt_custom_agents() -> Result<Vec<String>> {
    let input = Text::new("custom agents")
        .with_help_message("comma separated, e.g. cursor,qwen")
        .prompt()
        .context("failed to read custom agents")?;
    Ok(input
        .split(',')
        .map(str::trim)
        .filter(|agent| !agent.is_empty())
        .map(str::to_owned)
        .collect())
}

fn prompt_required(label: &str) -> Result<String> {
    let value = Text::new(label)
        .prompt()
        .with_context(|| format!("failed to read {label}"))?;
    if value.trim().is_empty() {
        bail!("{label} is required");
    }
    Ok(value.trim().to_owned())
}

fn prompt_confirm(question: &str, default: bool) -> Result<bool> {
    Confirm::new(question)
        .with_default(default)
        .prompt()
        .with_context(|| format!("failed to read confirmation for {question}"))
}

fn run_sksync(project_root: &PathBuf, args: &[String]) -> Result<()> {
    let exe = std::env::current_exe().context("failed to determine current executable")?;
    let status = Command::new(exe)
        .args(args)
        .current_dir(project_root)
        .status()
        .with_context(|| format!("failed to run sksync {}", args.join(" ")))?;
    if !status.success() {
        bail!("sksync {} failed with {status}", args.join(" "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{config_path_for_scope, ConfigScope};
    use std::path::Path;

    #[test]
    fn project_scope_uses_project_config_path() {
        assert_eq!(
            config_path_for_scope(Path::new("/tmp/project"), ConfigScope::Project).unwrap(),
            Path::new("/tmp/project/sksync.config.json")
        );
    }

    #[test]
    fn prompt_tui_module_is_available() {
        let run_fn: fn(std::path::PathBuf) -> anyhow::Result<()> = super::run;
        let _ = run_fn;
    }
}
