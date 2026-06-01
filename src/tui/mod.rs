use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::application::config::ResolvedConfig;
use crate::infrastructure::json::{
    default_agent_mapping_config, read_agent_mapping_config, AgentMappingConfig,
};
use anyhow::{bail, Context, Result};
use inquire::{Confirm, MultiSelect, Select, Text};

mod add_skill;
mod bundle;
mod commands;
mod config;
mod default_agents;
mod operations;
mod skill;

use config::{global_config_root, ConfigScope};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Intent {
    AddSkill,
    AttachAgent,
    RemoveSkill,
    RemoveAgent,
    AddBundle,
    RemoveBundle,
    Status,
    Apply,
    ConfigureDefaultAgents,
    Quit,
}

impl fmt::Display for Intent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::AddSkill => "Add skill",
            Self::AttachAgent => "Attach skill to agent",
            Self::RemoveSkill => "Remove skill",
            Self::RemoveAgent => "Detach skill from agent",
            Self::AddBundle => "Add bundle",
            Self::RemoveBundle => "Remove bundle",
            Self::Status => "Show status",
            Self::Apply => "Apply links",
            Self::ConfigureDefaultAgents => "Configure default agents",
            Self::Quit => "Quit",
        };
        formatter.write_str(label)
    }
}

fn wizard_intents() -> Vec<Intent> {
    vec![
        Intent::AddSkill,
        Intent::AttachAgent,
        Intent::RemoveSkill,
        Intent::RemoveAgent,
        Intent::AddBundle,
        Intent::RemoveBundle,
        Intent::Status,
        Intent::Apply,
        Intent::ConfigureDefaultAgents,
        Intent::Quit,
    ]
}

pub fn run(project_root: PathBuf) -> Result<()> {
    println!("sksync wizard");
    println!("Project: {}", project_root.display());

    loop {
        let intent = Select::new("What would you like to do?", wizard_intents())
            .prompt()
            .context("failed to read wizard selection")?;

        match intent {
            Intent::AddSkill => add_skill::run(&project_root)?,
            Intent::AttachAgent => skill::run_attach_agent(&project_root)?,
            Intent::RemoveSkill => skill::run_remove(&project_root)?,
            Intent::RemoveAgent => skill::run_remove_agent(&project_root)?,
            Intent::AddBundle => bundle::run_add(&project_root)?,
            Intent::RemoveBundle => bundle::run_remove(&project_root)?,
            Intent::Status => operations::run_status(&project_root)?,
            Intent::Apply => operations::run_apply(&project_root)?,
            Intent::ConfigureDefaultAgents => default_agents::run(&project_root)?,
            Intent::Quit => return Ok(()),
        }
    }
}

fn confirm_and_run(project_root: &Path, question: &str, args: Vec<String>) -> Result<()> {
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

fn prompt_skill_from_config(config: &ResolvedConfig, message: &str) -> Result<String> {
    let skills = configured_skill_names(config)?;
    Select::new(message, skills)
        .prompt()
        .context("failed to read skill selection")
}

fn prompt_skills_from_config(config: &ResolvedConfig, message: &str) -> Result<Vec<String>> {
    let selected = MultiSelect::new(message, configured_skill_names(config)?)
        .with_help_message("Use space to select, enter to confirm")
        .prompt()
        .context("failed to read skill selection")?;
    if selected.is_empty() {
        bail!("at least one skill is required");
    }
    Ok(selected)
}

fn configured_skill_names(config: &ResolvedConfig) -> Result<Vec<String>> {
    let skills = config
        .skills
        .iter()
        .map(|skill| skill.name.as_str().to_owned())
        .collect::<Vec<_>>();
    if skills.is_empty() {
        bail!("no skills are configured");
    }
    Ok(skills)
}

fn prompt_agents_not_for_skill(
    config: &ResolvedConfig,
    skill_name: &str,
    scope: ConfigScope,
) -> Result<Vec<String>> {
    let configured_agents = configured_agents_for_skill(config, skill_name)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    let available_agents = agent_options_for_scope(scope, Some(config))?
        .into_iter()
        .filter(|agent| !configured_agents.contains(agent))
        .collect::<Vec<_>>();
    if available_agents.is_empty() {
        bail!("skill {skill_name} is already attached to every available agent");
    }

    let selected = MultiSelect::new("Select agent(s) to attach to", available_agents)
        .with_help_message("Use space to select, enter to confirm")
        .prompt()
        .context("failed to read available agent selection")?;
    if selected.is_empty() {
        bail!("at least one agent is required");
    }
    Ok(selected)
}

fn prompt_agents_for_skill(config: &ResolvedConfig, skill_name: &str) -> Result<Vec<String>> {
    let agents = configured_agents_for_skill(config, skill_name)?;
    if agents.is_empty() {
        bail!("skill {skill_name} has no configured agents");
    }
    let selected = MultiSelect::new("Select agent(s) to detach from", agents)
        .with_help_message("Use space to select, enter to confirm")
        .prompt()
        .context("failed to read configured agent selection")?;
    if selected.is_empty() {
        bail!("at least one agent is required");
    }
    Ok(selected)
}

fn configured_agents_for_skill(config: &ResolvedConfig, skill_name: &str) -> Result<Vec<String>> {
    let skill = config
        .skills
        .iter()
        .find(|skill| skill.name.as_str() == skill_name)
        .with_context(|| format!("configured skill not found: {skill_name}"))?;
    Ok(skill
        .agents
        .iter()
        .map(|agent| agent.as_str().to_owned())
        .collect())
}

fn prompt_agents(
    scope: ConfigScope,
    config: Option<&ResolvedConfig>,
    default_agents: &[String],
) -> Result<Vec<String>> {
    let options = agent_options_for_scope(scope, config)?;
    let default_indexes = default_agent_indexes(&options, default_agents);
    let selected = MultiSelect::new("Select agent(s)", options)
        .with_default(&default_indexes)
        .with_help_message("Use space to select, enter to confirm")
        .prompt()
        .context("failed to read agent selection")?;

    normalize_agent_selection(selected, true)
}

fn prompt_default_agents(
    scope: ConfigScope,
    config: Option<&ResolvedConfig>,
    current_defaults: &[String],
) -> Result<Vec<String>> {
    let options = agent_options_for_scope(scope, config)?;
    let default_indexes = default_agent_indexes(&options, current_defaults);
    let selected = MultiSelect::new("Select default agent(s)", options)
        .with_default(&default_indexes)
        .with_help_message("Use space to select defaults, enter to save; empty clears defaults")
        .prompt()
        .context("failed to read default agent selection")?;

    normalize_agent_selection(selected, false)
}

fn normalize_agent_selection(
    mut agents: Vec<String>,
    require_non_empty: bool,
) -> Result<Vec<String>> {
    if require_non_empty && agents.is_empty() {
        bail!("at least one agent is required");
    }
    agents.sort();
    agents.dedup();
    Ok(agents)
}

fn default_agent_indexes(options: &[String], default_agents: &[String]) -> Vec<usize> {
    let defaults = default_agents.iter().collect::<BTreeSet<_>>();
    options
        .iter()
        .enumerate()
        .filter_map(|(index, agent)| defaults.contains(agent).then_some(index))
        .collect()
}

fn agent_options_for_scope(
    scope: ConfigScope,
    config: Option<&ResolvedConfig>,
) -> Result<Vec<String>> {
    Ok(merge_agent_options(
        scope,
        &merged_agent_mapping_config()?,
        config,
    ))
}

fn merge_agent_options(
    scope: ConfigScope,
    mappings: &AgentMappingConfig,
    config: Option<&ResolvedConfig>,
) -> Vec<String> {
    let mut agents = BTreeSet::new();
    agents.extend(mappings.global.keys().cloned());
    if scope == ConfigScope::Project {
        agents.extend(mappings.project.keys().cloned());
    }
    if let Some(config) = config {
        agents.extend(config.agents.keys().cloned());
    }
    agents.into_iter().collect()
}

fn merged_agent_mapping_config() -> Result<AgentMappingConfig> {
    let mut mappings = default_agent_mapping_config()?;
    let mapping_path = global_config_root()?.join("agents.json");
    if mapping_path.exists() {
        mappings.merge(read_agent_mapping_config(&mapping_path)?);
    }
    Ok(mappings)
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

fn run_sksync(project_root: &Path, args: &[String]) -> Result<()> {
    let previous_dir = std::env::current_dir().context("failed to determine current directory")?;
    std::env::set_current_dir(project_root).with_context(|| {
        format!(
            "failed to enter project directory {}",
            project_root.display()
        )
    })?;

    let result =
        crate::cli::run_with_args(std::iter::once("sksync".to_owned()).chain(args.iter().cloned()))
            .with_context(|| format!("failed to run sksync {}", args.join(" ")));

    let restore_result = std::env::set_current_dir(&previous_dir).with_context(|| {
        format!(
            "failed to restore working directory {}",
            previous_dir.display()
        )
    });

    match (result, restore_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(restore_error)) => Err(error.context(format!(
            "also failed to restore working directory: {restore_error}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::{default_agent_indexes, merge_agent_options, wizard_intents, ConfigScope, Intent};
    use crate::application::config::{ResolvedAgent, ResolvedConfig};
    use crate::domain::agent::AgentKind;
    use crate::domain::scope::Scope;
    use crate::domain::skill::SourcePath;
    use crate::infrastructure::json::AgentMappingConfig;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn wizard_intents_include_add_bundle() {
        assert!(wizard_intents().contains(&Intent::AddBundle));
    }

    #[test]
    fn wizard_intents_include_remove_bundle() {
        assert!(wizard_intents().contains(&Intent::RemoveBundle));
    }

    #[test]
    fn default_agent_indexes_match_available_options() {
        let options = vec![
            "claude-code".to_owned(),
            "pi".to_owned(),
            "universal".to_owned(),
        ];
        let defaults = vec![
            "universal".to_owned(),
            "missing".to_owned(),
            "pi".to_owned(),
        ];

        assert_eq!(default_agent_indexes(&options, &defaults), vec![1, 2]);
    }

    #[test]
    fn agent_options_include_inline_custom_config_agents() {
        let mappings = AgentMappingConfig {
            global: BTreeMap::from([("pi".to_owned(), PathBuf::from("~/.pi/agent/skills"))]),
            project: BTreeMap::new(),
        };
        let config = ResolvedConfig {
            skill_dir: SourcePath::new(".sksync/skills").expect("skill dir"),
            agents: BTreeMap::from([(
                "my-agent".to_owned(),
                ResolvedAgent {
                    kind: AgentKind::custom("my-agent").expect("custom agent"),
                    enabled: true,
                    scope: Scope::Project,
                    target_dir: Some(PathBuf::from(".my-agent/skills")),
                },
            )]),
            skills: Vec::new(),
            default_agents: vec![AgentKind::custom("my-agent").expect("custom agent")],
        };

        assert_eq!(
            merge_agent_options(ConfigScope::Project, &mappings, Some(&config)),
            vec!["my-agent", "pi"]
        );
    }

    #[test]
    fn prompt_tui_module_is_available() {
        let run_fn: fn(std::path::PathBuf) -> anyhow::Result<()> = super::run;
        let _ = run_fn;
    }
}
