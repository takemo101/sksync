use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use inquire::{Confirm, MultiSelect, Select, Text};
use serde_json::json;

use crate::application::bundle::load_bundle_from_source;
use crate::application::config::ResolvedConfig;
use crate::application::ports::ConfigStore;
use crate::infrastructure::json::{
    default_agent_mapping_config, read_agent_mapping_config, AgentMappingConfig, FileConfigStore,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Intent {
    AddSkill,
    AttachAgent,
    RemoveSkill,
    RemoveAgent,
    AddBundle,
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
            Self::Status => "Show status",
            Self::Apply => "Apply links",
            Self::ConfigureDefaultAgents => "Configure default agents",
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

fn wizard_intents() -> Vec<Intent> {
    vec![
        Intent::AddSkill,
        Intent::AttachAgent,
        Intent::RemoveSkill,
        Intent::RemoveAgent,
        Intent::AddBundle,
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
            Intent::AddSkill => run_add_flow(&project_root)?,
            Intent::AttachAgent => run_attach_agent_flow(&project_root)?,
            Intent::RemoveSkill => run_remove_flow(&project_root)?,
            Intent::RemoveAgent => run_remove_agent_flow(&project_root)?,
            Intent::AddBundle => run_add_bundle_flow(&project_root)?,
            Intent::Status => run_status_flow(&project_root)?,
            Intent::Apply => run_apply_flow(&project_root)?,
            Intent::ConfigureDefaultAgents => run_configure_default_agents_flow(&project_root)?,
            Intent::Quit => return Ok(()),
        }
    }
}

fn run_add_flow(project_root: &Path) -> Result<()> {
    let source = prompt_required("Skill source")?;
    let name = Text::new("Name override")
        .with_help_message("Optional; leave blank to infer from source")
        .prompt()
        .context("failed to read name override")?;
    let scope = prompt_config_scope("Where should this dependency be added?")?;
    let config = load_optional_config_for_scope(project_root, scope)?;
    let default_agents = default_agents_from_config(config.as_ref());
    let agents = prompt_agents(scope, config.as_ref(), &default_agents)?;
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

fn run_add_bundle_flow(project_root: &Path) -> Result<()> {
    let source = prompt_required("Bundle source")?;
    let scope = prompt_config_scope("Where should this bundle be added?")?;
    let root_dir = if scope.is_global() {
        global_config_root()?
    } else {
        project_root.to_path_buf()
    };
    let bundle = load_bundle_from_source(&source, &root_dir)?;

    println!("Bundle");
    println!("Name: {}", bundle.manifest.name);
    println!("Description: {}", bundle.manifest.description);
    println!("Source: {}", bundle.provenance.source);
    println!("Entries ({})", bundle.entries.len());
    for entry in &bundle.entries {
        println!(
            "- {}: {} -> {}",
            entry.skill_name, entry.original_source, entry.normalized_source
        );
    }
    if !prompt_confirm("Continue with this bundle?", true)? {
        return Ok(());
    }

    let config = load_optional_config_for_scope(project_root, scope)?;
    let default_agents = default_agents_from_config(config.as_ref());
    let agents = prompt_agents(scope, config.as_ref(), &default_agents)?;
    let dry_run_args = bundle_add_args(&source, &agents, scope.is_global(), true);
    println!("dry-run plan:");
    run_sksync(project_root, &dry_run_args)?;

    let apply_args = bundle_add_args(&source, &agents, scope.is_global(), false);
    confirm_and_run(project_root, "Add this bundle?", apply_args)
}

fn bundle_add_args(source: &str, agents: &[String], global: bool, dry_run: bool) -> Vec<String> {
    let mut args = vec!["bundle".to_owned(), "add".to_owned(), source.to_owned()];
    for agent in agents {
        args.push("--agent".to_owned());
        args.push(agent.clone());
    }
    if global {
        args.push("--global".to_owned());
    }
    if dry_run {
        args.push("--dry-run".to_owned());
    }
    args
}

fn run_attach_agent_flow(project_root: &Path) -> Result<()> {
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

fn run_remove_flow(project_root: &Path) -> Result<()> {
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

fn run_remove_agent_flow(project_root: &Path) -> Result<()> {
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

fn run_configure_default_agents_flow(project_root: &Path) -> Result<()> {
    let scope = prompt_config_scope("Which config should store default agents?")?;
    let config = load_optional_config_for_scope(project_root, scope)?;
    let current_defaults = default_agents_from_config(config.as_ref());
    let agents = prompt_default_agents(scope, config.as_ref(), &current_defaults)?;
    let config_path = config_path_for_scope(project_root, scope)?;
    write_default_agents_config(&config_path, default_skill_dir_for_scope(scope), &agents)?;
    println!("✓ Updated default agents in {}", config_path.display());
    Ok(())
}

fn run_status_flow(project_root: &Path) -> Result<()> {
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

fn run_apply_flow(project_root: &Path) -> Result<()> {
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

fn load_config_for_scope(project_root: &Path, scope: ConfigScope) -> Result<ResolvedConfig> {
    let path = config_path_for_scope(project_root, scope)?;
    if !path.exists() {
        bail!("config not found: {}", path.display());
    }
    FileConfigStore::new(path)
        .load()
        .context("failed to load config")
}

fn load_optional_config_for_scope(
    project_root: &Path,
    scope: ConfigScope,
) -> Result<Option<ResolvedConfig>> {
    let path = config_path_for_scope(project_root, scope)?;
    if !path.exists() {
        return Ok(None);
    }
    FileConfigStore::new(path)
        .load()
        .map(Some)
        .context("failed to load config")
}

fn config_path_for_scope(project_root: &Path, scope: ConfigScope) -> Result<PathBuf> {
    match scope {
        ConfigScope::Project => Ok(project_root.join("sksync.config.json")),
        ConfigScope::Global => Ok(global_config_root()?.join("config.json")),
    }
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

fn default_agents_from_config(config: Option<&ResolvedConfig>) -> Vec<String> {
    config
        .map(|config| {
            config
                .default_agents
                .iter()
                .map(|agent| agent.as_str().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn write_default_agents_config(
    config_path: &Path,
    default_skill_dir: &str,
    agents: &[String],
) -> Result<()> {
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut value = if config_path.exists() {
        serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(config_path)
                .with_context(|| format!("failed to read {}", config_path.display()))?,
        )
        .with_context(|| format!("failed to parse {}", config_path.display()))?
    } else {
        json!({
            "$schema": "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.schema.json",
            "skillDir": default_skill_dir,
            "dependencies": {}
        })
    };
    let object = value
        .as_object_mut()
        .context("config root must be a JSON object")?;
    object.insert("defaultAgents".to_owned(), json!(agents));
    std::fs::write(
        config_path,
        format!("{}\n", serde_json::to_string_pretty(&value)?),
    )
    .with_context(|| format!("failed to write {}", config_path.display()))
}

fn default_skill_dir_for_scope(scope: ConfigScope) -> &'static str {
    if scope.is_global() {
        "~/.sksync/skills"
    } else {
        "./.sksync/skills"
    }
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
    use super::{
        bundle_add_args, config_path_for_scope, default_agent_indexes, merge_agent_options,
        wizard_intents, write_default_agents_config, ConfigScope, Intent,
    };
    use crate::application::config::{ResolvedAgent, ResolvedConfig};
    use crate::domain::agent::AgentKind;
    use crate::domain::scope::Scope;
    use crate::domain::skill::SourcePath;
    use crate::infrastructure::json::AgentMappingConfig;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    #[test]
    fn wizard_intents_include_add_bundle() {
        assert!(wizard_intents().contains(&Intent::AddBundle));
    }

    #[test]
    fn bundle_add_args_include_agents_scope_and_dry_run() {
        assert_eq!(
            bundle_add_args(
                "./bundle",
                &["pi".to_owned(), "claude-code".to_owned()],
                true,
                true,
            ),
            vec![
                "bundle",
                "add",
                "./bundle",
                "--agent",
                "pi",
                "--agent",
                "claude-code",
                "--global",
                "--dry-run"
            ]
        );
    }

    #[test]
    fn project_scope_uses_project_config_path() {
        assert_eq!(
            config_path_for_scope(Path::new("/tmp/project"), ConfigScope::Project).unwrap(),
            Path::new("/tmp/project/sksync.config.json")
        );
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
    fn write_default_agents_config_creates_missing_config() {
        let temp = tempfile::tempdir().expect("temp dir");
        let config_path = temp.path().join("sksync.config.json");

        write_default_agents_config(
            &config_path,
            "./.sksync/skills",
            &["universal".to_owned(), "pi".to_owned()],
        )
        .expect("write defaults");

        let value = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(&config_path).expect("read config"),
        )
        .expect("parse config");
        assert_eq!(value["skillDir"], "./.sksync/skills");
        assert_eq!(value["dependencies"], serde_json::json!({}));
        assert_eq!(
            value["defaultAgents"],
            serde_json::json!(["universal", "pi"])
        );
    }

    #[test]
    fn write_default_agents_config_preserves_existing_config_fields() {
        let temp = tempfile::tempdir().expect("temp dir");
        let config_path = temp.path().join("sksync.config.json");
        std::fs::write(
            &config_path,
            r#"{
              "skillDir": "skills",
              "dependencies": {
                "review": { "source": "./review", "agents": ["pi"] }
              }
            }"#,
        )
        .expect("write config");

        write_default_agents_config(&config_path, "./.sksync/skills", &["universal".to_owned()])
            .expect("write defaults");

        let value = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(&config_path).expect("read config"),
        )
        .expect("parse config");
        assert_eq!(value["skillDir"], "skills");
        assert_eq!(
            value["dependencies"]["review"]["agents"],
            serde_json::json!(["pi"])
        );
        assert_eq!(value["defaultAgents"], serde_json::json!(["universal"]));
    }

    #[test]
    fn prompt_tui_module_is_available() {
        let run_fn: fn(std::path::PathBuf) -> anyhow::Result<()> = super::run;
        let _ = run_fn;
    }
}
