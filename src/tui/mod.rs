use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use inquire::{Confirm, MultiSelect, Select, Text};

use crate::application::config::ResolvedConfig;
use crate::application::ports::ConfigStore;
use crate::infrastructure::json::{
    default_agent_mapping_config, read_agent_mapping_config, AgentMappingConfig, FileConfigStore,
};

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
            Self::AddSkill => "Add skill",
            Self::RemoveSkill => "Remove skill",
            Self::RemoveAgent => "Detach skill from agent",
            Self::Status => "Show status",
            Self::Apply => "Apply links",
            Self::Quit => "Quit",
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
            Self::Global => "global config (~/.sksync/config.json)",
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
            Self::Normal => "Normal removal (no option; removes symlinks too)",
            Self::KeepFiles => "Keep installed skill files (--keep-files)",
            Self::ConfigOnly => "Config and lockfile only (--config-only)",
        };
        formatter.write_str(label)
    }
}

pub fn run(project_root: PathBuf) -> Result<()> {
    println!("sksync wizard");
    println!("Project: {}", project_root.display());

    loop {
        let intent = Select::new(
            "What would you like to do?",
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
        .context("failed to read wizard selection")?;

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
    let source = prompt_required("Skill source")?;
    let name = Text::new("Name override")
        .with_help_message("Optional; leave blank to infer from source")
        .prompt()
        .context("failed to read name override")?;
    let scope = prompt_config_scope("Where should this dependency be added?")?;
    let agents = prompt_agents(scope)?;
    let global = scope.is_global();

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

    confirm_and_run(project_root, "Run this command?", args)
}

fn run_remove_flow(project_root: &PathBuf) -> Result<()> {
    let scope = prompt_config_scope("Which config should the skill be removed from?")?;
    let config = load_config_for_scope(project_root, scope)?;
    let skill = prompt_skill_from_config(&config, "Select the skill to remove")?;
    let mode = prompt_remove_mode()?;

    let mut args = vec!["remove".to_owned(), skill];
    if scope.is_global() {
        args.push("--global".to_owned());
    }
    mode.append_args(&mut args);

    confirm_and_run(project_root, "Remove this skill?", args)
}

fn run_remove_agent_flow(project_root: &PathBuf) -> Result<()> {
    let scope = prompt_config_scope("Which config should be updated?")?;
    let config = load_config_for_scope(project_root, scope)?;
    let skill = prompt_skill_from_config(&config, "Select the skill to detach from an agent")?;
    let agents = prompt_agents_for_skill(&config, &skill)?;

    let mut args = vec!["remove".to_owned(), skill];
    for agent in agents {
        args.push("--agent".to_owned());
        args.push(agent);
    }
    if scope.is_global() {
        args.push("--global".to_owned());
    }

    confirm_and_run(
        project_root,
        "Detach this skill from the selected agent(s)?",
        args,
    )
}

fn run_status_flow(project_root: &PathBuf) -> Result<()> {
    let global = prompt_config_scope("Which config should be inspected?")?.is_global();
    let check = prompt_confirm("Run check after list?", true)?;

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
    let global = prompt_config_scope("Which config should be applied?")?.is_global();
    let force = prompt_confirm("Allow safe replacement of managed links?", false)?;

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

    confirm_and_run(project_root, "Apply these link changes?", apply_args)
}

fn confirm_and_run(project_root: &PathBuf, question: &str, args: Vec<String>) -> Result<()> {
    println!("Planned command: sksync {}", args.join(" "));
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
        "Select remove mode",
        vec![
            RemoveMode::Normal,
            RemoveMode::KeepFiles,
            RemoveMode::ConfigOnly,
        ],
    )
    .with_help_message("Normal removal is the same as CLI `sksync remove <skill>`")
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
    MultiSelect::new("Select agent(s) to detach from", agents)
        .with_help_message("Use space to select, enter to confirm")
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
        ConfigScope::Global => Ok(global_config_root()?.join("config.json")),
    }
}

fn prompt_agents(scope: ConfigScope) -> Result<Vec<String>> {
    let selected = MultiSelect::new("Select agent(s)", agent_options_for_scope(scope)?)
        .with_help_message("Use space to select, enter to confirm")
        .prompt()
        .context("failed to read agent selection")?;

    let mut agents = selected;

    if agents.is_empty() {
        bail!("at least one agent is required");
    }
    agents.sort();
    agents.dedup();
    Ok(agents)
}

fn agent_options_for_scope(scope: ConfigScope) -> Result<Vec<String>> {
    let mappings = merged_agent_mapping_config()?;
    let mut agents = BTreeSet::new();
    agents.extend(mappings.global.keys().cloned());
    if scope == ConfigScope::Project {
        agents.extend(mappings.project.keys().cloned());
    }
    Ok(agents.into_iter().collect())
}

fn merged_agent_mapping_config() -> Result<AgentMappingConfig> {
    let mut mappings = default_agent_mapping_config()?;
    let mapping_path = global_config_root()?.join("agents.json");
    if mapping_path.exists() {
        mappings.merge(read_agent_mapping_config(&mapping_path)?);
    }
    Ok(mappings)
}

fn global_config_root() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|dir| dir.join(".sksync"))
        .context("failed to determine home directory for global sksync directory")
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
