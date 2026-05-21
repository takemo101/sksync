use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as GitCommand;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::application::apply::{apply_link_plan, ApplyOptions};
use crate::application::check::check_lockfile;
use crate::application::config::{apply_agent_target_dirs, ResolvedConfig};
use crate::application::init::{init_global, init_project};
use crate::application::list::list_skills;
use crate::application::outdated::{collect_outdated, RemoteRefError, RemoteRefResolver};
use crate::application::plan::build_link_plan;
use crate::application::ports::{DependencyConfigStore, LockfileStore};
use crate::application::update::update_dependencies;
use crate::domain::agent::AgentKind;
use crate::domain::link_plan::LinkPlan;
use crate::domain::lockfile::{LockedFile, LockedSkill, Lockfile};
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
    /// Create a starter sksync config and skills directory.
    Init(InitArgs),
    /// Add a dependency, update it, and apply symlinks.
    Add(AddArgs),
    /// Remove a dependency, installed skill, managed symlinks, and lock entry.
    Remove(RemoveArgs),
    /// Show dependencies that can be updated.
    Outdated(OutdatedArgs),
    /// Show the synchronization plan without changing the filesystem.
    Plan(PlanArgs),
    /// Apply the synchronization plan to the filesystem.
    Apply(ApplyArgs),
    /// Recreate skills from sksync-lock.json when present, then apply symlinks.
    Install(InstallArgs),
    /// Download latest dependency skills and refresh sksync-lock.json.
    Update(UpdateArgs),
    /// Check config, lockfile, hashes, and symlink health.
    Check(CheckArgs),
    /// List managed skills and agent link status.
    List(ListArgs),
    /// Launch the interactive prompt wizard.
    #[command(visible_aliases = ["ask", "tui"])]
    Wizard,
}

#[derive(Debug, Args)]
struct InitArgs {
    /// Initialize ~/.config/sksync/config.json instead of ./sksync.config.json.
    #[arg(long)]
    global: bool,
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
struct RemoveArgs {
    /// Skill name to remove.
    skill: String,
    /// Use ~/.config/sksync/config.json instead of project config.
    #[arg(long)]
    global: bool,
    /// Remove only from config and lockfile, leaving installed files and symlinks untouched.
    #[arg(long)]
    config_only: bool,
    /// Remove the skill only from the specified agent. Can be passed multiple times.
    #[arg(long = "agent")]
    agents: Vec<String>,
    /// Keep the installed skill directory under skillDir.
    #[arg(long)]
    keep_files: bool,
}

#[derive(Debug, Args)]
struct OutdatedArgs {
    /// Use ~/.config/sksync/config.json instead of project config.
    #[arg(long)]
    global: bool,
    /// Print machine-readable JSON.
    #[arg(long)]
    json: bool,
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
struct InstallArgs {
    /// Use ~/.config/sksync/config.json and global lockfile instead of project files.
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
        Command::Init(args) => run_init(args),
        Command::Add(args) => run_add(args),
        Command::Remove(args) => run_remove(args),
        Command::Outdated(args) => run_outdated(args),
        Command::Plan(args) => run_plan(args),
        Command::Apply(args) => run_apply(args),
        Command::Install(args) => run_install(args),
        Command::Update(args) => run_update(args),
        Command::Check(args) => run_check(args),
        Command::List(args) => run_list(args),
        Command::Wizard => run_wizard(),
    }
}

fn run_init(args: InitArgs) -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let result = if args.global {
        init_global(config_root_for_global()?)?
    } else {
        init_project(&current_dir)?
    };
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

    let mut config = load_config_from_path(&config_path, scope_for(args.global))?;
    let report = update_dependencies(&config, &FileSystemSkillInstaller)?;
    apply_update_report_sources(&mut config, &report);
    print_update_report(report);
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

fn run_remove(args: RemoveArgs) -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let config_path = config_path_for(args.global, &current_dir)?;
    let config = load_config_for_scope(args.global, &current_dir)?;
    let skill = config
        .skills
        .iter()
        .find(|skill| skill.name.as_str() == args.skill);
    let skill_source = skill.map(|skill| skill.source.as_path().to_path_buf());
    let lockfile_path = lockfile_path_for(args.global, &current_dir)?;
    let mut lockfile = read_lockfile(&lockfile_path).ok();
    let (_config, removal_plan, _root_dir) =
        build_plan_from_config(config.clone(), args.global, &current_dir)?;

    if args.agents.is_empty() {
        remove_entire_skill(
            &args,
            &config_path,
            skill_source,
            &lockfile_path,
            &mut lockfile,
            &removal_plan,
        )?;
        return Ok(());
    }

    let requested_agents = parse_agent_kinds(&args.agents)?;
    let removes_all_agents = skill
        .map(|skill| {
            !skill.agents.is_empty()
                && skill
                    .agents
                    .iter()
                    .all(|agent| requested_agents.iter().any(|requested| requested == agent))
        })
        .unwrap_or(false);

    if removes_all_agents {
        remove_entire_skill(
            &args,
            &config_path,
            skill_source,
            &lockfile_path,
            &mut lockfile,
            &removal_plan,
        )?;
        return Ok(());
    }

    remove_skill_agents(
        &args,
        &config_path,
        &lockfile_path,
        &mut lockfile,
        &requested_agents,
        &removal_plan,
    )
}

fn remove_entire_skill(
    args: &RemoveArgs,
    config_path: &Path,
    skill_source: Option<PathBuf>,
    lockfile_path: &Path,
    lockfile: &mut Option<Lockfile>,
    removal_plan: &LinkPlan,
) -> Result<()> {
    if !args.config_only {
        remove_managed_symlinks(removal_plan, &args.skill)?;
        if !args.keep_files {
            if let Some(source) = skill_source {
                if source.exists() {
                    fs::remove_dir_all(&source).with_context(|| {
                        format!("failed to remove installed skill {}", source.display())
                    })?;
                    println!("Removed {}", source.display());
                }
            }
        }
    }

    FileDependencyConfigStore::new(config_path, default_skill_dir_for(args.global)?)
        .remove_dependency(&args.skill)?;
    if let Some(lockfile) = lockfile {
        if let Ok(skill_name) = crate::domain::skill::SkillName::new(args.skill.clone()) {
            lockfile.skills.remove(&skill_name);
            FileLockfileStore::new(lockfile_path).write(lockfile)?;
        }
    }
    println!("Removed {}", args.skill);
    Ok(())
}

fn remove_skill_agents(
    args: &RemoveArgs,
    config_path: &Path,
    lockfile_path: &Path,
    lockfile: &mut Option<Lockfile>,
    requested_agents: &[AgentKind],
    removal_plan: &LinkPlan,
) -> Result<()> {
    if !args.config_only {
        remove_managed_symlinks_for_agents(removal_plan, &args.skill, requested_agents)?;
    }

    let requested_agent_names = requested_agents
        .iter()
        .map(|agent| agent.as_str().to_owned())
        .collect::<Vec<_>>();
    let remaining_agents =
        FileDependencyConfigStore::new(config_path, default_skill_dir_for(args.global)?)
            .remove_dependency_agents(&args.skill, &requested_agent_names)?;

    if let Some(lockfile) = lockfile {
        if let Ok(skill_name) = crate::domain::skill::SkillName::new(args.skill.clone()) {
            if let Some(locked) = lockfile.skills.get_mut(&skill_name) {
                locked
                    .targets
                    .retain(|target| !agent_kinds_contain(requested_agents, &target.agent));
            }
            FileLockfileStore::new(lockfile_path).write(lockfile)?;
        }
    }

    println!(
        "Removed {} from agent(s): {}",
        args.skill,
        requested_agent_names.join(", ")
    );
    if remaining_agents.is_empty() {
        println!(
            "No agents remain for {}; removed dependency entry",
            args.skill
        );
    }
    Ok(())
}

fn parse_agent_kinds(agents: &[String]) -> Result<Vec<AgentKind>> {
    agents
        .iter()
        .map(|agent| {
            AgentKind::from_str(agent).with_context(|| format!("invalid agent name {agent:?}"))
        })
        .collect()
}

fn agent_kinds_contain(agents: &[AgentKind], agent: &AgentKind) -> bool {
    agents.iter().any(|candidate| candidate == agent)
}

fn remove_managed_symlinks(plan: &LinkPlan, skill: &str) -> Result<()> {
    for item in &plan.items {
        if item.skill.as_str() == skill {
            remove_managed_symlink_target(item.source.as_path(), item.target.as_path())?;
        }
    }
    Ok(())
}

fn remove_managed_symlinks_for_agents(
    plan: &LinkPlan,
    skill: &str,
    agents: &[AgentKind],
) -> Result<()> {
    for item in &plan.items {
        if item.skill.as_str() == skill && agent_kinds_contain(agents, &item.agent) {
            remove_managed_symlink_target(item.source.as_path(), item.target.as_path())?;
        }
    }
    Ok(())
}

fn remove_managed_symlink_target(source: &Path, target: &Path) -> Result<()> {
    match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            if symlink_points_to_locked_source(target, source)? {
                fs::remove_file(target)
                    .with_context(|| format!("failed to remove symlink {}", target.display()))?;
                println!("Removed symlink {}", target.display());
            } else {
                println!(
                    "Skipped symlink not pointing to locked source {}",
                    target.display()
                );
            }
        }
        Ok(_) => {
            println!("Skipped non-symlink target {}", target.display());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect target {}", target.display()))
        }
    }
    Ok(())
}

fn symlink_points_to_locked_source(target: &Path, source: &Path) -> Result<bool> {
    let actual = fs::read_link(target)
        .with_context(|| format!("failed to read symlink {}", target.display()))?;
    if actual == source {
        return Ok(true);
    }
    let actual_abs = if actual.is_absolute() {
        actual
    } else {
        target
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(actual)
    };
    let source_abs = if source.is_absolute() {
        source.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to determine current directory")?
            .join(source)
    };
    Ok(actual_abs == source_abs
        || (actual_abs.exists()
            && source_abs.exists()
            && fs::canonicalize(actual_abs)? == fs::canonicalize(source_abs)?))
}

fn run_outdated(args: OutdatedArgs) -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let config = load_config_for_scope(args.global, &current_dir)?;
    let lockfile_path = lockfile_path_for(args.global, &current_dir)?;
    let lockfile = read_lockfile(&lockfile_path)?;
    let report = collect_outdated(&config, &lockfile, &GitRemoteRefResolver);
    let rows = report.rows;
    if args.json {
        let json_rows = rows
            .iter()
            .map(|row| {
                serde_json::json!({
                    "skill": row.skill,
                    "current": row.current,
                    "wanted": row.wanted,
                    "latest": row.latest,
                    "source": row.source,
                    "status": row.status,
                })
            })
            .collect::<Vec<_>>();
        println!("{}", serde_json::to_string_pretty(&json_rows)?);
    } else if rows.is_empty() {
        println!("All skills are up to date.");
    } else {
        println!("Skill\tCurrent\tWanted\tLatest\tSource\tStatus");
        for row in rows {
            println!(
                "{}\t{}\t{}\t{}\t{}\t{}",
                row.skill, row.current, row.wanted, row.latest, row.source, row.status
            );
        }
    }
    Ok(())
}

struct GitRemoteRefResolver;

impl RemoteRefResolver for GitRemoteRefResolver {
    fn git_remote_rev(&self, repo: &str, reference: &str) -> Result<String, RemoteRefError> {
        let output = GitCommand::new("git")
            .arg("ls-remote")
            .arg(repo)
            .arg(reference)
            .output()
            .map_err(|error| RemoteRefError::Query(error.to_string()))?;
        if !output.status.success() {
            return Err(RemoteRefError::Query(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout
            .split_whitespace()
            .next()
            .map(str::to_owned)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                RemoteRefError::Query(format!("no revision found for {repo} {reference}"))
            })
    }
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

fn run_install(args: InstallArgs) -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let mut config = load_config_for_scope(args.global, &current_dir)?;
    let lockfile_path = lockfile_path_for(args.global, &current_dir)?;
    if lockfile_path.exists() {
        let lockfile = read_lockfile(&lockfile_path)?;
        apply_locked_install_sources(&mut config, &lockfile);
    }
    let report = update_dependencies(&config, &FileSystemSkillInstaller)?;
    apply_update_report_sources(&mut config, &report);
    print_update_report(report);
    let (config, plan, root_dir) = build_plan_from_config(config, args.global, &current_dir)?;
    let lockfile = build_lockfile_from_plan(&config, &plan, &root_dir)?;
    let fs_store = FileSystemLinkStore;
    let lockfile_store = FileLockfileStore::new(lockfile_path);
    apply_link_plan(
        &plan,
        &lockfile,
        &fs_store,
        &lockfile_store,
        ApplyOptions { force: false },
    )?;
    print_plan(&plan);
    println!("Wrote sksync-lock.json");
    Ok(())
}

fn run_update(args: UpdateArgs) -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let mut config = load_config_for_scope(args.global, &current_dir)?;
    let report = update_dependencies(&config, &FileSystemSkillInstaller)?;
    apply_update_report_sources(&mut config, &report);
    print_update_report(report);
    let (config, plan, root_dir) = build_plan_from_config(config, args.global, &current_dir)?;
    let lockfile = build_lockfile_from_plan(&config, &plan, &root_dir)?;
    FileLockfileStore::new(lockfile_path_for(args.global, &current_dir)?).write(&lockfile)?;
    println!("Wrote sksync-lock.json");
    Ok(())
}

fn apply_update_report_sources(
    config: &mut ResolvedConfig,
    report: &crate::application::update::UpdateReport,
) {
    for updated in &report.updated {
        if let Some(skill) = config
            .skills
            .iter_mut()
            .find(|skill| skill.name.as_str() == updated.name)
        {
            skill.install_source = Some(updated.resolved_source.clone());
        }
    }
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

fn apply_locked_install_sources(config: &mut ResolvedConfig, lockfile: &Lockfile) {
    for skill in &mut config.skills {
        if let Some(locked) = lockfile.skills.get(&skill.name) {
            if let Some(install_source) = &locked.install_source {
                skill.install_source = Some(install_source.clone());
            }
        }
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
        Ok(PathBuf::from("./.sksync/skills"))
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
    _plan: &LinkPlan,
    current_dir: &std::path::Path,
) -> Result<Lockfile> {
    let mut skills = BTreeMap::new();

    for skill in &config.skills {
        let hash = hash_directory(skill.source.as_path())
            .with_context(|| format!("failed to hash {}", skill.source.as_path().display()))?;
        skills.insert(
            skill.name.clone(),
            LockedSkill {
                source: skill.source.clone(),
                install_source: skill.install_source.clone(),
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
            },
        );
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

fn run_wizard() -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    crate::tui::run(current_dir)
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::{CommandFactory, Parser};

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
            [
                "init", "add", "remove", "outdated", "plan", "apply", "install", "update", "check",
                "list", "wizard",
            ]
        );
    }

    #[test]
    fn init_help_is_available() {
        Cli::command()
            .try_get_matches_from(["sksync", "init", "--help"])
            .expect_err("--help should short-circuit as a clap display error");
    }

    #[test]
    fn init_global_is_registered() {
        Cli::try_parse_from(["sksync", "init", "--global"]).expect("init --global should parse");
    }

    #[test]
    fn plan_help_is_available() {
        Cli::command()
            .try_get_matches_from(["sksync", "plan", "--help"])
            .expect_err("--help should short-circuit as a clap display error");
    }

    #[test]
    fn wizard_aliases_are_registered() {
        Cli::try_parse_from(["sksync", "wizard"]).expect("wizard should parse");
        Cli::try_parse_from(["sksync", "ask"]).expect("ask alias should parse");
        Cli::try_parse_from(["sksync", "tui"]).expect("tui alias should parse");
    }
}
