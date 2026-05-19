use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::application::apply::{apply_link_plan, ApplyOptions};
use crate::application::check::check_lockfile;
use crate::application::config::{apply_agent_target_dirs, ResolvedConfig};
use crate::application::init::init_project;
use crate::application::list::list_skills;
use crate::application::plan::build_link_plan;
use crate::application::ports::DependencyConfigStore;
use crate::application::update::update_dependencies;
use crate::domain::link_plan::LinkPlan;
use crate::domain::lockfile::{LinkType, LockedFile, LockedSkill, LockedTarget, Lockfile};
use crate::domain::scope::Scope;
use crate::infrastructure::builtin_agents::TargetPathResolver;
use crate::infrastructure::fs::FileSystemLinkStore;
use crate::infrastructure::hash::{hash_directory, Sha256SourceHashStore};
use crate::infrastructure::install::FileSystemSkillInstaller;
use crate::infrastructure::json::{
    read_agent_mappings, read_lockfile, FileConfigStore, FileDependencyConfigStore,
    FileLockfileStore,
};
use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};

/// sksync command line interface.
#[derive(Debug, Parser)]
#[command(
    name = "sksync",
    version,
    about = "Synchronize AI agent skill symlinks"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a starter sksync.config.json and skills directory.
    Init,
    /// Add a dependency, update it, and apply symlinks.
    Add(AddArgs),
    /// Show the synchronization plan without changing the filesystem.
    Plan(PlanArgs),
    /// Apply the synchronization plan to the filesystem.
    Apply(ApplyArgs),
    /// Download or refresh dependency skills into skillDir.
    Update(UpdateArgs),
    /// Check config, lockfile, hashes, and symlink health.
    Check(CheckArgs),
    /// List managed skills and agent link status.
    List(ListArgs),
    /// Launch the interactive terminal UI.
    Tui,
}

#[derive(Debug, Args)]
struct AddArgs {
    /// Skill source, e.g. owner/repo/path#ref, github:owner/repo/path#ref, registry:skills.sh/owner/skill, or ./local-skill.
    source: String,
    /// Agent to link into. Can be passed multiple times.
    #[arg(short, long = "agent", required = true)]
    agents: Vec<String>,
    /// Override inferred skill name.
    #[arg(long)]
    name: Option<String>,
    /// Write ~/.config/sksync/config.json instead of ./sksync.config.json.
    #[arg(long)]
    global: bool,
}

#[derive(Debug, Args)]
struct PlanArgs {
    /// Explicitly run in dry-run mode.
    #[arg(long)]
    dry_run: bool,
    /// Use ~/.config/sksync/config.json instead of project config.
    #[arg(long)]
    global: bool,
}

#[derive(Debug, Args)]
struct ApplyArgs {
    /// Allow replacing existing sksync-managed links when it is safe to do so.
    #[arg(long)]
    force: bool,
    /// Use ~/.config/sksync/config.json instead of project config.
    #[arg(long)]
    global: bool,
}

#[derive(Debug, Args)]
struct UpdateArgs {
    /// Use ~/.config/sksync/config.json instead of project config.
    #[arg(long)]
    global: bool,
}

#[derive(Debug, Args)]
struct ListArgs {
    /// Use ~/.config/sksync/config.json instead of project config.
    #[arg(long)]
    global: bool,
}

#[derive(Debug, Args)]
struct CheckArgs {
    /// Use ~/.config/sksync/sksync-lock.json instead of project lockfile.
    #[arg(long)]
    global: bool,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    dispatch(cli.command)
}

fn dispatch(command: Command) -> Result<()> {
    match command {
        Command::Init => run_init(),
        Command::Add(args) => run_add(args),
        Command::Plan(args) => run_plan(args),
        Command::Apply(args) => run_apply(args),
        Command::Update(args) => run_update(args),
        Command::Check(args) => run_check(args),
        Command::List(args) => run_list(args),
        Command::Tui => run_tui(),
    }
}

fn run_init() -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let result = init_project(&current_dir)?;
    println!("Created {}", result.config_path.display());
    println!("Created {}", result.skills_dir.display());
    Ok(())
}

fn run_add(args: AddArgs) -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let config_path = config_path_for(args.global, &current_dir)?;
    let skill_name = args.name.unwrap_or_else(|| infer_skill_name(&args.source));

    FileDependencyConfigStore::new(&config_path, default_skill_dir_for(args.global)?)
        .add_dependency(&skill_name, &args.source, &args.agents)?;
    println!("Added {skill_name} to {}", config_path.display());

    let config = load_config_from_path(&config_path, scope_for(args.global))?;
    print_update_report(update_dependencies(&config, &FileSystemSkillInstaller)?);
    let (config, plan, root_dir) = build_plan_from_config(config, args.global, &current_dir)?;
    let lockfile = build_lockfile_from_plan(&config, &plan, &root_dir)?;
    let fs_store = FileSystemLinkStore;
    let lockfile_store = FileLockfileStore::new(lockfile_path_for(args.global, &current_dir)?);
    apply_link_plan(
        &plan,
        &lockfile,
        &fs_store,
        &lockfile_store,
        ApplyOptions { force: false },
    )?;
    print_plan(&plan);
    Ok(())
}

fn run_plan(args: PlanArgs) -> Result<()> {
    let (_config, plan, _current_dir) = load_plan(args.global)?;
    print_plan(&plan);
    Ok(())
}

fn run_apply(args: ApplyArgs) -> Result<()> {
    let (config, plan, current_dir) = load_plan(args.global)?;
    let lockfile = build_lockfile_from_plan(&config, &plan, &current_dir)?;
    let fs_store = FileSystemLinkStore;
    let lockfile_store = FileLockfileStore::new(lockfile_path_for(args.global, &current_dir)?);

    apply_link_plan(
        &plan,
        &lockfile,
        &fs_store,
        &lockfile_store,
        ApplyOptions { force: args.force },
    )?;
    print_plan(&plan);
    println!("Wrote sksync-lock.json");

    Ok(())
}

fn run_update(args: UpdateArgs) -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let config = load_config_for_scope(args.global, &current_dir)?;
    print_update_report(update_dependencies(&config, &FileSystemSkillInstaller)?);
    Ok(())
}

fn print_update_report(report: crate::application::update::UpdateReport) {
    for updated in report.updated {
        println!(
            "Updated {} from {} -> {}",
            updated.name,
            updated.source,
            updated.destination.display()
        );
    }
    for skipped in report.skipped {
        println!("Skipped {skipped}: no dependency source");
    }
}

fn load_plan(global: bool) -> Result<(ResolvedConfig, LinkPlan, PathBuf)> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let config = load_config_for_scope(global, &current_dir)?;
    build_plan_from_config(config, global, &current_dir)
}

fn build_plan_from_config(
    config: ResolvedConfig,
    global: bool,
    current_dir: &Path,
) -> Result<(ResolvedConfig, LinkPlan, PathBuf)> {
    let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    let root_dir = if global {
        config_root_for_global()?
    } else {
        current_dir.to_path_buf()
    };
    let fs_store = FileSystemLinkStore;
    let target_resolver = TargetPathResolver::new(&root_dir, home_dir);
    let plan = build_link_plan(&config, &fs_store, &fs_store, &target_resolver)?;

    Ok((config, plan, root_dir))
}

fn load_config(current_dir: &Path) -> Result<ResolvedConfig> {
    load_config_for_scope(false, current_dir)
}

fn load_config_for_scope(global: bool, current_dir: &Path) -> Result<ResolvedConfig> {
    let config_path = config_path_for(global, current_dir)?;
    load_config_from_path(&config_path, scope_for(global))
}

fn load_config_from_path(config_path: &Path, default_scope: Scope) -> Result<ResolvedConfig> {
    let mut config = FileConfigStore::new(config_path).load_with_default_scope(default_scope)?;
    if let Some(config_dir) = dirs::config_dir() {
        let mapping_path = config_dir.join("sksync/agents.json");
        if mapping_path.exists() {
            let mappings = read_agent_mappings(&mapping_path)?;
            apply_agent_target_dirs(&mut config, mappings)?;
        }
    }
    Ok(config)
}

fn scope_for(global: bool) -> Scope {
    if global {
        Scope::User
    } else {
        Scope::Project
    }
}

fn config_path_for(global: bool, current_dir: &Path) -> Result<PathBuf> {
    if global {
        Ok(config_root_for_global()?.join("config.json"))
    } else {
        Ok(current_dir.join("sksync.config.json"))
    }
}

fn lockfile_path_for(global: bool, current_dir: &Path) -> Result<PathBuf> {
    if global {
        Ok(config_root_for_global()?.join("sksync-lock.json"))
    } else {
        Ok(current_dir.join("sksync-lock.json"))
    }
}

fn config_root_for_global() -> Result<PathBuf> {
    dirs::config_dir()
        .map(|dir| dir.join("sksync"))
        .context("failed to determine global config directory")
}

fn default_skill_dir_for(global: bool) -> Result<PathBuf> {
    if global {
        Ok(config_root_for_global()?.join("skills"))
    } else {
        Ok(PathBuf::from("./skills"))
    }
}

fn infer_skill_name(source: &str) -> String {
    let without_ref = source.split('#').next().unwrap_or(source);
    let trimmed = without_ref.trim_end_matches('/');
    trimmed
        .rsplit('/')
        .find(|part| !part.is_empty() && *part != "tree")
        .unwrap_or("skill")
        .trim_end_matches(".git")
        .to_owned()
}

fn print_plan(plan: &LinkPlan) {
    if plan.is_empty() {
        println!("No actions planned.");
    } else {
        for line in plan.display_lines() {
            println!("{line}");
        }
    }
}

fn build_lockfile_from_plan(
    config: &ResolvedConfig,
    plan: &LinkPlan,
    current_dir: &std::path::Path,
) -> Result<Lockfile> {
    let mut skills = BTreeMap::new();

    for item in &plan.items {
        let hash = hash_directory(item.source.as_path())
            .with_context(|| format!("failed to hash {}", item.source.as_path().display()))?;
        let entry = skills
            .entry(item.skill.clone())
            .or_insert_with(|| LockedSkill {
                source: item.source.clone(),
                hash: hash.hash.clone(),
                files: hash
                    .files
                    .iter()
                    .map(|file| LockedFile {
                        path: file.path.clone(),
                        hash: file.hash.clone(),
                    })
                    .collect(),
                targets: Vec::new(),
            });
        let scope = config
            .agents
            .get(item.agent.as_str())
            .map(|agent| agent.scope)
            .context("planned agent is missing from resolved config")?;
        entry.targets.push(LockedTarget {
            agent: item.agent.clone(),
            scope,
            path: item.target.clone(),
            link_type: LinkType::Symlink,
        });
    }

    Ok(Lockfile {
        generated_by: format!("sksync@{}", env!("CARGO_PKG_VERSION")),
        generated_at: generated_at(),
        root: current_dir.to_path_buf(),
        skills,
    })
}

fn generated_at() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| format!("unix:{}", duration.as_secs()))
        .unwrap_or_else(|_| "unix:0".to_owned())
}

fn run_check(args: CheckArgs) -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let lockfile = read_lockfile(lockfile_path_for(args.global, &current_dir)?)?;
    let report = check_lockfile(&lockfile, &Sha256SourceHashStore, &FileSystemLinkStore);

    for line in report.display_lines() {
        println!("{line}");
    }

    if report.is_success() {
        Ok(())
    } else {
        bail!("check found {} problem(s)", report.problems.len())
    }
}

fn run_list(args: ListArgs) -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    let config = load_config_for_scope(args.global, &current_dir)?;
    let root_dir = if args.global {
        config_root_for_global()?
    } else {
        current_dir.clone()
    };
    let lockfile = read_lockfile(lockfile_path_for(args.global, &current_dir)?).ok();
    let target_resolver = TargetPathResolver::new(&root_dir, home_dir);
    let report = list_skills(
        &config,
        lockfile.as_ref(),
        &FileSystemLinkStore,
        &target_resolver,
    );

    for line in report.display_lines() {
        println!("{line}");
    }

    Ok(())
}

fn run_tui() -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let app = crate::tui::app::TuiApp::new(
        current_dir.clone(),
        current_dir.join("sksync.config.json").exists(),
        current_dir.join("sksync-lock.json").exists(),
    );
    crate::tui::run(app)
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn help_mentions_binary_name() {
        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("sksync"));
    }

    #[test]
    fn subcommands_are_registered() {
        let command = Cli::command();
        let names = command
            .get_subcommands()
            .map(|subcommand| subcommand.get_name().to_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            ["init", "add", "plan", "apply", "update", "check", "list", "tui",]
        );
    }

    #[test]
    fn init_help_is_available() {
        Cli::command()
            .try_get_matches_from(["sksync", "init", "--help"])
            .expect_err("--help should short-circuit as a clap display error");
    }

    #[test]
    fn plan_help_is_available() {
        Cli::command()
            .try_get_matches_from(["sksync", "plan", "--help"])
            .expect_err("--help should short-circuit as a clap display error");
    }
}
