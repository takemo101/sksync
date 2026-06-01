use std::fmt;
use std::path::Path;

use anyhow::{Context, Result};
use inquire::Select;

use super::config::load_config_for_scope;
use super::{
    confirm_and_run, prompt_agents_for_skill, prompt_agents_not_for_skill, prompt_config_scope,
    prompt_skill_from_config, prompt_skills_from_config,
};

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

pub(super) fn run_attach_agent(project_root: &Path) -> Result<()> {
    let scope = prompt_config_scope("Which config should be updated?")?;
    let config = load_config_for_scope(project_root, scope)?;
    let skill = prompt_skill_from_config(&config, "Select the skill to attach to agent(s)")?;
    let agents = prompt_agents_not_for_skill(&config, &skill, scope)?;

    let mut args = vec!["attach".to_owned(), skill];
    for agent in agents {
        args.push("--agent".to_owned());
        args.push(agent);
    }
    if scope.is_global() {
        args.push("--global".to_owned());
    }

    confirm_and_run(
        project_root,
        "Attach this skill to the selected agent(s)?",
        args,
    )
}

pub(super) fn run_remove(project_root: &Path) -> Result<()> {
    let scope = prompt_config_scope("Which config should the skill be removed from?")?;
    let config = load_config_for_scope(project_root, scope)?;
    let skills = prompt_skills_from_config(&config, "Select skill(s) to remove")?;
    let mode = prompt_remove_mode()?;

    let mut args = vec!["remove".to_owned()];
    args.extend(skills);
    if scope.is_global() {
        args.push("--global".to_owned());
    }
    mode.append_args(&mut args);

    confirm_and_run(project_root, "Remove this skill?", args)
}

pub(super) fn run_remove_agent(project_root: &Path) -> Result<()> {
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
