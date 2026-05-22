use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command as GitCommand;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::application::add::{run_add_workflow, AddSelection, AddWorkflow};
use crate::application::apply::{apply_link_plan, ApplyOptions};
use crate::application::check::{check_lockfile_with_plan, CheckProblem};
use crate::application::config::{apply_agent_target_mappings, AgentTargetDir, ResolvedConfig};
use crate::application::discovery::{
    discover_source_skills, infer_skill_name, source_with_selected_subpath, SkillCandidate,
};
use crate::application::init::{init_agents, init_global, init_project};
use crate::application::list::{list_skills, ListReport, ListedTargetState};
use crate::application::outdated::{
    collect_outdated, OutdatedRow, RemoteRefError, RemoteRefResolver,
};
use crate::application::plan::{build_desired_link_plan, build_link_plan};
use crate::application::ports::{DependencyConfigStore, LockfileStore};
use crate::application::update::{apply_update_report_sources, update_dependencies};
use crate::domain::agent::AgentKind;
use crate::domain::link_plan::{LinkPlan, LinkPlanItem, PlanAction};
use crate::domain::lockfile::{LockedFile, LockedSkill, Lockfile};
use crate::domain::scope::Scope;
use crate::infrastructure::builtin_agents::TargetPathResolver;
use crate::infrastructure::fs::FileSystemLinkStore;
use crate::infrastructure::hash::{hash_directory, Sha256SourceHashStore};
use crate::infrastructure::install::FileSystemSkillInstaller;
use crate::infrastructure::json::{
    default_agent_mapping_config, parse_install_source_value, read_agent_mapping_config,
    read_lockfile, AgentMappingConfig, FileConfigStore, FileDependencyConfigStore,
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
    /// Initialize ~/.sksync/config.json instead of ./sksync.config.json.
    #[arg(long)]
    global: bool,
    /// Force overwrite ~/.sksync/agents.json with bundled agent mappings only.
    #[arg(long)]
    agents: bool,
}

#[derive(Debug, Args)]
struct AddArgs {
    /// Skill source, e.g. owner/repo/path#ref, github:owner/repo/path#ref, skills.sh/owner/repo/skill-name#ref, https://www.skills.sh/owner/repo/skill-name#ref, or ./local-skill.
    source: String,
    /// Agent to link into. Can be passed multiple times.
    #[arg(short, long = "agent", required = true)]
    agents: Vec<String>,
    /// Override inferred skill name.
    #[arg(long)]
    name: Option<String>,
    /// Write ~/.sksync/config.json instead of ./sksync.config.json.
    #[arg(long)]
    global: bool,
}

#[derive(Debug, Args)]
struct RemoveArgs {
    /// Skill name to remove.
    skill: String,
    /// Use ~/.sksync/config.json instead of project config.
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
    /// Use ~/.sksync/config.json instead of project config.
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
    /// Use ~/.sksync/config.json instead of project config.
    #[arg(long)]
    global: bool,
}

#[derive(Debug, Args)]
struct ApplyArgs {
    /// Allow replacing existing sksync-managed links when it is safe to do so.
    #[arg(long)]
    force: bool,
    /// Use ~/.sksync/config.json instead of project config.
    #[arg(long)]
    global: bool,
}

#[derive(Debug, Args)]
struct InstallArgs {
    /// Use ~/.sksync/config.json and global lockfile instead of project files.
    #[arg(long)]
    global: bool,
}

#[derive(Debug, Args)]
struct UpdateArgs {
    /// Use ~/.sksync/config.json instead of project config.
    #[arg(long)]
    global: bool,
}

#[derive(Debug, Args)]
struct ListArgs {
    /// Use ~/.sksync/config.json instead of project config.
    #[arg(long)]
    global: bool,
}

#[derive(Debug, Args)]
struct CheckArgs {
    /// Use ~/.sksync/sksync-lock.json instead of project lockfile.
    #[arg(long)]
    global: bool,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    dispatch(cli.command)
}

pub(crate) fn run_with_args<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::try_parse_from(args)?;
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
    if args.agents {
        let result = init_agents(config_root_for_global()?)?;
        print_success(format!(
            "Updated agent mappings: {}",
            result.agent_mapping_path.display()
        ));
        return Ok(());
    }

    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let result = if args.global {
        init_global(config_root_for_global()?)?
    } else {
        init_project(&current_dir)?
    };
    print_success(format!("Created config: {}", result.config_path.display()));
    if let Some(agent_mapping_path) = result.agent_mapping_path {
        print_success(format!(
            "Created agent mappings: {}",
            agent_mapping_path.display()
        ));
    }
    print_success(format!(
        "Created skills directory: {}",
        result.skills_dir.display()
    ));
    Ok(())
}

fn run_add(args: AddArgs) -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let config_path = config_path_for(args.global, &current_dir)?;
    reject_legacy_registry_source(&args.source)?;
    let selections = resolve_add_selections(&args.source, args.name.as_deref(), &config_path)?;
    let config_backup = ConfigFileBackup::capture(&config_path)?;
    let add_result = (|| -> Result<()> {
        let store =
            FileDependencyConfigStore::new(&config_path, default_skill_dir_for(args.global)?);
        let fs_store = FileSystemLinkStore;
        let lockfile_store = FileLockfileStore::new(lockfile_path_for(args.global, &current_dir)?);
        let root_dir = if args.global {
            config_root_for_global()?
        } else {
            current_dir.clone()
        };
        let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
        let target_resolver = TargetPathResolver::new(&root_dir, home_dir);
        let report = run_add_workflow(
            selections,
            &args.agents,
            || load_config_from_path(&config_path, scope_for(args.global)),
            |config, plan| build_lockfile_from_plan(config, plan, &root_dir),
            AddWorkflow {
                dependency_store: &store,
                installer: &FileSystemSkillInstaller,
                fs_store: &fs_store,
                lockfile_store: &lockfile_store,
                target_resolver: &target_resolver,
            },
        )?;
        for added in &report.added {
            print_success(format!(
                "Added dependency: {} ({})",
                added.skill_name,
                config_path.display()
            ));
        }
        print_update_report(report.update_report);
        print_plan(&report.plan);
        Ok(())
    })();

    if let Err(error) = add_result {
        if let Err(restore_error) = config_backup.restore() {
            return Err(error.context(format!(
                "sksync add failed and config rollback failed: {restore_error}"
            )));
        }
        return Err(error.context("sksync add failed; restored previous config"));
    }

    Ok(())
}

struct ConfigFileBackup {
    path: PathBuf,
    content: Option<Vec<u8>>,
}

impl ConfigFileBackup {
    fn capture(path: &Path) -> Result<Self> {
        let content = if path.exists() {
            Some(
                fs::read(path)
                    .with_context(|| format!("failed to read config backup {}", path.display()))?,
            )
        } else {
            None
        };

        Ok(Self {
            path: path.to_path_buf(),
            content,
        })
    }

    fn restore(&self) -> Result<()> {
        match &self.content {
            Some(content) => fs::write(&self.path, content)
                .with_context(|| format!("failed to restore config {}", self.path.display())),
            None => {
                if self.path.exists() {
                    fs::remove_file(&self.path).with_context(|| {
                        format!(
                            "failed to remove rolled-back config {}",
                            self.path.display()
                        )
                    })?;
                }
                Ok(())
            }
        }
    }
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
    let skill_dir = config.skill_dir.as_path().to_path_buf();
    let lockfile_path = lockfile_path_for(args.global, &current_dir)?;
    let mut lockfile = read_lockfile(&lockfile_path).ok();
    let (_config, removal_plan, _root_dir) =
        build_plan_from_config(config.clone(), args.global, &current_dir)?;

    if args.agents.is_empty() {
        remove_entire_skill(
            &args,
            &config_path,
            skill_source,
            &skill_dir,
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
            &skill_dir,
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
    skill_dir: &Path,
    lockfile_path: &Path,
    lockfile: &mut Option<Lockfile>,
    removal_plan: &LinkPlan,
) -> Result<()> {
    if !args.config_only {
        remove_managed_symlinks(removal_plan, &args.skill)?;
        if !args.keep_files {
            if let Some(source) = skill_source {
                remove_installed_skill_dir(&source, skill_dir)?;
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
    print_success(format!("Removed skill: {}", args.skill));
    Ok(())
}

fn remove_installed_skill_dir(source: &Path, skill_dir: &Path) -> Result<()> {
    if !source.exists() {
        return Ok(());
    }

    if !is_managed_skill_dir(source, skill_dir)? {
        print_info(format!(
            "Skipped unmanaged skill files: {}",
            source.display()
        ));
        return Ok(());
    }

    fs::remove_dir_all(source)
        .with_context(|| format!("failed to remove installed skill {}", source.display()))?;
    print_success(format!(
        "Removed installed skill files: {}",
        source.display()
    ));
    Ok(())
}

fn is_managed_skill_dir(source: &Path, skill_dir: &Path) -> Result<bool> {
    if !source.is_dir() || !skill_dir.is_dir() || source == skill_dir {
        return Ok(false);
    }

    let canonical_source = source
        .canonicalize()
        .with_context(|| format!("failed to resolve installed skill {}", source.display()))?;
    let canonical_skill_dir = skill_dir
        .canonicalize()
        .with_context(|| format!("failed to resolve skillDir {}", skill_dir.display()))?;

    Ok(canonical_source.starts_with(&canonical_skill_dir)
        && canonical_source != canonical_skill_dir)
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

    print_success(format!(
        "Detached {} from agent(s): {}",
        args.skill,
        requested_agent_names.join(", ")
    ));
    if remaining_agents.is_empty() {
        print_info(format!(
            "No agents remain for {}; removed dependency entry",
            args.skill
        ));
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
                print_success(format!("Removed symlink: {}", target.display()));
            } else {
                print_info(format!(
                    "Skipped symlink not pointing to locked source: {}",
                    target.display()
                ));
            }
        }
        Ok(_) => {
            print_info(format!("Skipped non-symlink target: {}", target.display()));
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
        print_success("All skills are up to date.");
    } else {
        print_outdated_rows(&rows);
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
    print_lockfile_written(lockfile_path_for(args.global, &current_dir)?);

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
    print_lockfile_written(lockfile_path_for(args.global, &current_dir)?);
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
    let lockfile_path = lockfile_path_for(args.global, &current_dir)?;
    FileLockfileStore::new(&lockfile_path).write(&lockfile)?;
    print_lockfile_written(lockfile_path);
    Ok(())
}

fn print_update_report(report: crate::application::update::UpdateReport) {
    if report.updated.is_empty() && report.skipped.is_empty() {
        print_info("No dependency updates.");
        return;
    }

    for updated in report.updated {
        print_success(format!("Updated skill: {}", updated.name));
        print_detail(format!("source: {}", updated.source));
        print_detail(format!("destination: {}", updated.destination.display()));
    }
    for skipped in report.skipped {
        print_info(format!("Skipped {skipped}: no dependency source"));
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
    let mappings = agent_target_mappings_for_scope(default_scope)?;
    apply_agent_target_mappings(&mut config, mappings)?;
    Ok(config)
}

fn agent_target_mappings_for_scope(scope: Scope) -> Result<BTreeMap<String, AgentTargetDir>> {
    Ok(agent_target_mappings_from_config(
        merged_agent_mapping_config()?,
        scope,
    ))
}

fn agent_target_mappings_from_config(
    mapping_config: AgentMappingConfig,
    scope: Scope,
) -> BTreeMap<String, AgentTargetDir> {
    let mut mappings = BTreeMap::new();

    for (name, target_dir) in mapping_config.global {
        mappings.insert(
            name,
            AgentTargetDir {
                target_dir,
                scope: Scope::User,
            },
        );
    }

    if scope == Scope::Project {
        for (name, target_dir) in mapping_config.project {
            mappings.insert(
                name,
                AgentTargetDir {
                    target_dir,
                    scope: Scope::Project,
                },
            );
        }
    }

    mappings
}

fn merged_agent_mapping_config() -> Result<crate::infrastructure::json::AgentMappingConfig> {
    let mut mappings = default_agent_mapping_config()?;
    let mapping_path = config_root_for_global()?.join("agents.json");
    if mapping_path.exists() {
        mappings.merge(read_agent_mapping_config(&mapping_path)?);
    }
    Ok(mappings)
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
    dirs::home_dir()
        .map(|dir| global_config_root_from_home(&dir))
        .context("failed to determine home directory for global sksync directory")
}

fn global_config_root_from_home(home_dir: &Path) -> PathBuf {
    home_dir.join(".sksync")
}

fn default_skill_dir_for(global: bool) -> Result<PathBuf> {
    if global {
        Ok(PathBuf::from("~/.sksync/skills"))
    } else {
        Ok(PathBuf::from("./.sksync/skills"))
    }
}

#[derive(Debug, Clone)]
struct SkillChoice(SkillCandidate);

impl std::fmt::Display for SkillChoice {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}  {}",
            self.0.name,
            self.0.relative_path.display()
        )
    }
}

fn format_selected_skill_choices(
    selected: &[inquire::list_option::ListOption<&SkillChoice>],
) -> String {
    let names = selected
        .iter()
        .map(|option| option.value.0.name.as_str())
        .collect::<Vec<_>>();

    match names.as_slice() {
        [] => "no skills".to_owned(),
        [name] => (*name).to_owned(),
        _ if names.len() <= 4 => format!("{} skills: {}", names.len(), names.join(", ")),
        _ => format!("{} skills: {}, …", names.len(), names[..4].join(", ")),
    }
}

fn score_skill_choice(
    input: &str,
    choice: &SkillChoice,
    _display: &str,
    _index: usize,
) -> Option<i64> {
    let filter = input.trim().to_lowercase();
    if filter.is_empty() {
        return Some(0);
    }

    let name = choice.0.name.to_lowercase();
    let path = choice.0.relative_path.to_string_lossy().to_lowercase();
    let description = choice.0.description.to_lowercase();

    if name.contains(&filter) {
        Some(100)
    } else if path.contains(&filter) {
        Some(50)
    } else if description.contains(&filter) {
        Some(10)
    } else {
        None
    }
}

fn resolve_add_selections(
    source: &str,
    requested_name: Option<&str>,
    config_path: &Path,
) -> Result<Vec<AddSelection>> {
    let config_root = config_path.parent().unwrap_or_else(|| Path::new("."));
    let fallback_name = infer_skill_name(source);
    let parse_skill_name = requested_name.unwrap_or(&fallback_name);
    let install_source = parse_install_source_value(parse_skill_name, source, Some(config_root))
        .with_context(|| format!("failed to parse source '{source}'"))?;

    let discovered = discover_source_skills(&install_source, source)?;
    let selection_name = requested_name.or(discovered.default_selection_name.as_deref());
    let selections = select_skill_candidates(source, selection_name, discovered.candidates)?;

    Ok(selections
        .into_iter()
        .map(|selection| AddSelection {
            skill_name: requested_name
                .map(str::to_owned)
                .unwrap_or_else(|| selection.name.clone()),
            source: source_with_selected_subpath(
                source,
                &selection.relative_path,
                discovered.rewrite_mode,
            ),
        })
        .collect())
}

fn select_skill_candidates(
    source: &str,
    requested_name: Option<&str>,
    candidates: Vec<SkillCandidate>,
) -> Result<Vec<SkillCandidate>> {
    if candidates.is_empty() {
        bail!("no SKILL.md files found under source '{source}'");
    }

    if let Some(name) = requested_name {
        let matches = candidates
            .iter()
            .filter(|candidate| {
                candidate.name == name
                    || candidate
                        .relative_path
                        .file_name()
                        .and_then(|file_name| file_name.to_str())
                        == Some(name)
            })
            .cloned()
            .collect::<Vec<_>>();
        return match matches.as_slice() {
            [candidate] => Ok(vec![candidate.clone()]),
            [] => bail!("no discovered skill named '{name}' under source '{source}'"),
            _ => bail!("multiple discovered skills matched '{name}' under source '{source}'"),
        };
    }

    if candidates.len() == 1 {
        return Ok(vec![candidates.into_iter().next().expect("one candidate")]);
    }

    if !std::io::stdin().is_terminal() {
        bail!(
            "multiple skills found under source '{source}'; pass --name <skill> or use a more specific source"
        );
    }

    let choices = candidates.into_iter().map(SkillChoice).collect::<Vec<_>>();
    let selected = inquire::MultiSelect::new("Select skills to add", choices)
        .with_formatter(&format_selected_skill_choices)
        .with_scorer(&score_skill_choice)
        .with_help_message("space: select · type: filter name/path/description · enter: confirm")
        .prompt()?;
    if selected.is_empty() {
        bail!("no skills selected");
    }
    Ok(selected.into_iter().map(|choice| choice.0).collect())
}

fn reject_legacy_registry_source(source: &str) -> Result<()> {
    let body = source.split('#').next().unwrap_or(source).trim();
    if body.starts_with("registry:") {
        bail!(
            "registry sources are not supported; use a provider URL such as https://www.skills.sh/owner/repo/skill-name"
        );
    }
    Ok(())
}

fn print_plan(plan: &LinkPlan) {
    if plan.is_empty() {
        print_success("Link plan is clean. No changes needed.");
        return;
    }

    print_section_with_count("Link plan", plan.items.len());
    for item in &plan.items {
        print_plan_item(item);
    }
}

fn print_plan_item(item: &LinkPlanItem) {
    let (badge, title) = plan_action_badge(&item.action);
    println!("{badge:<8} {} → {}", item.skill, item.agent.as_str());
    print_detail(format!("action: {title}"));
    match &item.action {
        PlanAction::CreateSymlink | PlanAction::AlreadySynced => {
            print_detail(format!("target: {}", item.target.as_path().display()));
            print_detail(format!("source: {}", item.source.as_path().display()));
        }
        PlanAction::Conflict { reason } => {
            print_detail(format!("target: {}", item.target.as_path().display()));
            print_detail(format!("reason: {reason}"));
        }
        PlanAction::DriftedSymlink { actual_source } => {
            print_detail(format!("target: {}", item.target.as_path().display()));
            print_detail(format!("actual: {}", actual_source.display()));
            print_detail(format!("expected: {}", item.source.as_path().display()));
        }
        PlanAction::SourceMissing => {
            print_detail(format!("source: {}", item.source.as_path().display()));
            print_detail(format!("target: {}", item.target.as_path().display()));
        }
    }
}

fn plan_action_badge(action: &PlanAction) -> (&'static str, &'static str) {
    match action {
        PlanAction::CreateSymlink => ("CREATE", "create managed symlink"),
        PlanAction::AlreadySynced => ("OK", "already synced"),
        PlanAction::Conflict { .. } => ("BLOCKED", "target conflict"),
        PlanAction::DriftedSymlink { .. } => ("DRIFT", "symlink points elsewhere"),
        PlanAction::SourceMissing => ("MISSING", "source directory missing"),
    }
}

fn print_outdated_rows(rows: &[OutdatedRow]) {
    print_section_with_count("Outdated skills", rows.len());
    let table_rows = rows
        .iter()
        .map(|row| {
            vec![
                row.skill.clone(),
                compact_revision(&row.current),
                compact_revision(&row.wanted),
                compact_revision(&row.latest),
                compact_source(&row.source),
                row.status.clone(),
            ]
        })
        .collect::<Vec<_>>();
    print_table(
        &["Skill", "Current", "Wanted", "Latest", "Source", "Status"],
        &table_rows,
    );

    for row in rows.iter().filter(|row| outdated_row_needs_detail(row)) {
        print_detail(format!("{} current: {}", row.skill, row.current));
        print_detail(format!("{} wanted: {}", row.skill, row.wanted));
        print_detail(format!("{} latest: {}", row.skill, row.latest));
        print_detail(format!("{} source: {}", row.skill, row.source));
    }
}

fn outdated_row_needs_detail(row: &OutdatedRow) -> bool {
    compact_revision(&row.current) != row.current
        || compact_revision(&row.wanted) != row.wanted
        || compact_revision(&row.latest) != row.latest
        || compact_source(&row.source) != row.source
}

fn print_skill_list(report: &ListReport) {
    if report.skills.is_empty() {
        print_info("No skills configured.");
        return;
    }

    print_section_with_count("Skills", report.skills.len());
    for skill in &report.skills {
        println!("• {}", skill.name);
        if let Some(hash) = &skill.locked_hash {
            print_detail(format!("locked: {}", compact_revision(hash)));
        }
        if skill.targets.is_empty() {
            print_detail("no enabled targets");
            continue;
        }
        for target in &skill.targets {
            let path = if target.target.as_os_str().is_empty() {
                "unresolved".to_owned()
            } else {
                target.target.display().to_string()
            };
            println!(
                "  {} {:<14} {:<15} {}",
                list_state_icon(&target.state),
                target.agent,
                list_state_label(&target.state),
                path
            );
            if let Some(message) = list_state_detail(&target.state) {
                print_detail(message);
            }
        }
    }
}

fn list_state_icon(state: &ListedTargetState) -> &'static str {
    match state {
        ListedTargetState::Synced => "✓",
        ListedTargetState::Missing | ListedTargetState::SourceMissing => "○",
        ListedTargetState::Drifted
        | ListedTargetState::Conflict
        | ListedTargetState::BrokenSymlink
        | ListedTargetState::InspectFailed(_)
        | ListedTargetState::ResolveFailed(_) => "!",
    }
}

fn list_state_label(state: &ListedTargetState) -> &'static str {
    match state {
        ListedTargetState::Missing => "missing",
        ListedTargetState::Synced => "synced",
        ListedTargetState::Drifted => "drifted",
        ListedTargetState::Conflict => "conflict",
        ListedTargetState::BrokenSymlink => "broken",
        ListedTargetState::SourceMissing => "source-missing",
        ListedTargetState::InspectFailed(_) => "inspect-failed",
        ListedTargetState::ResolveFailed(_) => "resolve-failed",
    }
}

fn list_state_detail(state: &ListedTargetState) -> Option<String> {
    match state {
        ListedTargetState::InspectFailed(message) | ListedTargetState::ResolveFailed(message) => {
            Some(format!("reason: {message}"))
        }
        _ => None,
    }
}

fn print_check_problems(problems: &[CheckProblem]) {
    print_section_with_count("Check problems", problems.len());
    for problem in problems {
        println!("✗ {}", problem.display_line());
    }
}

fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    let mut widths = headers
        .iter()
        .map(|header| header.len())
        .collect::<Vec<_>>();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(index) {
                *width = (*width).max(cell.len());
            }
        }
    }

    print_table_row(headers.iter().copied(), &widths);
    let separators = widths
        .iter()
        .map(|width| "─".repeat(*width))
        .collect::<Vec<_>>();
    print_table_row(separators.iter().map(String::as_str), &widths);
    for row in rows {
        print_table_row(row.iter().map(String::as_str), &widths);
    }
}

fn print_table_row<'a>(cells: impl IntoIterator<Item = &'a str>, widths: &[usize]) {
    let cells = cells.into_iter().collect::<Vec<_>>();
    for (index, cell) in cells.iter().enumerate() {
        if index > 0 {
            print!("  ");
        }
        print!("{cell:<width$}", width = widths[index]);
    }
    println!();
}

fn print_lockfile_written(path: impl AsRef<Path>) {
    print_success(format!("Wrote lockfile: {}", path.as_ref().display()));
}

fn print_section(label: &str) {
    println!("\n{label}");
    println!("{}", "─".repeat(label.chars().count()));
}

fn print_section_with_count(label: &str, count: usize) {
    let heading = format!("{label} ({count})");
    print_section(&heading);
}

fn print_success(message: impl AsRef<str>) {
    println!("✓ {}", message.as_ref());
}

fn print_info(message: impl AsRef<str>) {
    println!("ℹ {}", message.as_ref());
}

fn print_detail(message: impl AsRef<str>) {
    println!("  {}", message.as_ref());
}

fn compact_revision(value: &str) -> String {
    if value.starts_with("error:") {
        truncate_middle(value, 48)
    } else if is_hash_like(value) {
        value.chars().take(12).collect()
    } else {
        truncate_middle(value, 18)
    }
}

fn compact_source(value: &str) -> String {
    truncate_middle(value, 42)
}

fn is_hash_like(value: &str) -> bool {
    value.len() >= 20 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn truncate_middle(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars || max_chars <= 1 {
        return value.to_owned();
    }

    let keep = max_chars.saturating_sub(1);
    let front = keep / 2;
    let back = keep - front;
    let prefix = value.chars().take(front).collect::<String>();
    let suffix = value
        .chars()
        .rev()
        .take(back)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{prefix}…{suffix}")
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
    let config = load_config_for_scope(args.global, &current_dir)?;
    let root_dir = if args.global {
        config_root_for_global()?
    } else {
        current_dir.clone()
    };
    let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    let target_resolver = TargetPathResolver::new(&root_dir, home_dir);
    let plan = build_desired_link_plan(&config, &target_resolver)?;
    let report = check_lockfile_with_plan(
        &lockfile,
        &plan,
        &Sha256SourceHashStore,
        &FileSystemLinkStore,
    );

    if report.is_success() {
        print_success("Check passed. Config, lockfile, hashes, and links are healthy.");
        Ok(())
    } else {
        print_check_problems(&report.problems);
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

    print_skill_list(&report);

    Ok(())
}

fn run_wizard() -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    crate::tui::run(current_dir)
}

#[cfg(test)]
mod tests {
    use super::{
        agent_target_mappings_from_config, compact_revision, compact_source,
        format_selected_skill_choices, global_config_root_from_home, is_managed_skill_dir,
        list_state_label, reject_legacy_registry_source, remove_installed_skill_dir,
        score_skill_choice, select_skill_candidates, truncate_middle, Cli, ConfigFileBackup,
    };
    use crate::application::discovery::{
        discover_skill_candidates, source_with_selected_subpath, SourceRewriteMode,
    };
    use crate::domain::scope::Scope;
    use crate::infrastructure::json::AgentMappingConfig;
    use clap::{CommandFactory, Parser};
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};

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
    fn init_agents_is_registered() {
        Cli::try_parse_from(["sksync", "init", "--agents"]).expect("init --agents should parse");
    }

    #[test]
    fn add_provider_option_is_not_registered() {
        assert!(Cli::try_parse_from([
            "sksync",
            "add",
            "owner/repo/skills/review#main",
            "--provider",
            "skills.sh",
            "--agent",
            "pi",
        ])
        .is_err());
    }

    #[test]
    fn add_rejects_legacy_registry_source_before_writing_config() {
        assert!(reject_legacy_registry_source("registry:skills.sh/owner/repo/skill#main").is_err());
    }

    #[test]
    fn discovers_skill_directories_under_source() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        std::fs::create_dir_all(root.join("skills/find-skills")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/ignored")).unwrap();
        std::fs::write(
            root.join("skills/find-skills/SKILL.md"),
            "---\nname: find-skills\ndescription: Find skills\n---\n# Find skills\n",
        )
        .unwrap();
        std::fs::write(
            root.join("node_modules/ignored/SKILL.md"),
            "---\nname: ignored\ndescription: Ignored\n---\n# Ignored\n",
        )
        .unwrap();

        let candidates = discover_skill_candidates(&root, 5).unwrap();

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].name, "find-skills");
        assert_eq!(candidates[0].relative_path, Path::new("skills/find-skills"));
    }

    #[test]
    fn config_file_backup_restores_existing_config() {
        let temp = tempfile::tempdir().expect("temp dir");
        let config_path = temp.path().join("sksync.config.json");
        fs::write(&config_path, "{\"dependencies\":{}}\n").expect("write config");
        let backup = ConfigFileBackup::capture(&config_path).expect("capture backup");
        fs::write(&config_path, "{\"dependencies\":{\"bad\":{}}}\n").expect("mutate config");

        backup.restore().expect("restore backup");

        assert_eq!(
            fs::read_to_string(&config_path).expect("read config"),
            "{\"dependencies\":{}}\n"
        );
    }

    #[test]
    fn config_file_backup_removes_created_config() {
        let temp = tempfile::tempdir().expect("temp dir");
        let config_path = temp.path().join("sksync.config.json");
        let backup = ConfigFileBackup::capture(&config_path).expect("capture missing backup");
        fs::write(&config_path, "{\"dependencies\":{\"bad\":{}}}\n").expect("create config");

        backup.restore().expect("restore missing backup");

        assert!(!config_path.exists());
    }

    #[test]
    fn managed_skill_dir_must_be_inside_skill_dir() {
        let temp = tempfile::tempdir().expect("temp dir");
        let skill_dir = temp.path().join(".sksync/skills");
        let managed = skill_dir.join("review");
        let outside = temp.path().join("outside/review");
        fs::create_dir_all(&managed).expect("create managed skill");
        fs::create_dir_all(&outside).expect("create outside skill");

        assert!(is_managed_skill_dir(&managed, &skill_dir).expect("check managed"));
        assert!(!is_managed_skill_dir(&outside, &skill_dir).expect("check outside"));
        assert!(!is_managed_skill_dir(&skill_dir, &skill_dir).expect("check root"));
    }

    #[test]
    fn remove_installed_skill_dir_skips_unmanaged_source() {
        let temp = tempfile::tempdir().expect("temp dir");
        let skill_dir = temp.path().join(".sksync/skills");
        let outside = temp.path().join("outside/review");
        fs::create_dir_all(&skill_dir).expect("create skill dir");
        fs::create_dir_all(&outside).expect("create outside skill");
        fs::write(
            outside.join("SKILL.md"),
            "---\nname: review\ndescription: Review\n---\n",
        )
        .expect("write outside skill");

        remove_installed_skill_dir(&outside, &skill_dir).expect("remove skips unmanaged");

        assert!(outside.exists());
        assert!(outside.join("SKILL.md").exists());
    }

    #[test]
    fn remove_installed_skill_dir_skips_when_skill_dir_is_missing() {
        let temp = tempfile::tempdir().expect("temp dir");
        let missing_skill_dir = temp.path().join(".sksync/skills");
        let outside = temp.path().join("outside/review");
        fs::create_dir_all(&outside).expect("create outside skill");

        remove_installed_skill_dir(&outside, &missing_skill_dir)
            .expect("missing skillDir should not block remove");

        assert!(outside.exists());
    }

    #[test]
    fn remove_installed_skill_dir_removes_managed_source() {
        let temp = tempfile::tempdir().expect("temp dir");
        let skill_dir = temp.path().join(".sksync/skills");
        let managed = skill_dir.join("review");
        fs::create_dir_all(&managed).expect("create managed skill");
        fs::write(
            managed.join("SKILL.md"),
            "---\nname: review\ndescription: Review\n---\n",
        )
        .expect("write managed skill");

        remove_installed_skill_dir(&managed, &skill_dir).expect("remove managed");

        assert!(!managed.exists());
    }

    #[test]
    fn skill_choice_display_is_compact() {
        let choice = super::SkillChoice(super::SkillCandidate {
            name: "review".to_owned(),
            description: "Review helper with a long explanation".to_owned(),
            relative_path: PathBuf::from("skills/review"),
        });

        assert_eq!(choice.to_string(), "review  skills/review");
        assert!(!choice.to_string().contains("long explanation"));
    }

    #[test]
    fn selected_skill_formatter_summarizes_many_choices() {
        let choices = [
            super::SkillChoice(super::SkillCandidate {
                name: "one".to_owned(),
                description: "First".to_owned(),
                relative_path: PathBuf::from("skills/one"),
            }),
            super::SkillChoice(super::SkillCandidate {
                name: "two".to_owned(),
                description: "Second".to_owned(),
                relative_path: PathBuf::from("skills/two"),
            }),
            super::SkillChoice(super::SkillCandidate {
                name: "three".to_owned(),
                description: "Third".to_owned(),
                relative_path: PathBuf::from("skills/three"),
            }),
            super::SkillChoice(super::SkillCandidate {
                name: "four".to_owned(),
                description: "Fourth".to_owned(),
                relative_path: PathBuf::from("skills/four"),
            }),
            super::SkillChoice(super::SkillCandidate {
                name: "five".to_owned(),
                description: "Fifth".to_owned(),
                relative_path: PathBuf::from("skills/five"),
            }),
        ];
        let selected = choices
            .iter()
            .enumerate()
            .map(|(index, choice)| inquire::list_option::ListOption::new(index, choice))
            .collect::<Vec<_>>();

        assert_eq!(
            format_selected_skill_choices(&selected),
            "5 skills: one, two, three, four, …"
        );
    }

    #[test]
    fn skill_choice_scorer_searches_description_without_displaying_it() {
        let choice = super::SkillChoice(super::SkillCandidate {
            name: "diagnose".to_owned(),
            description: "Hard bugs and performance regressions".to_owned(),
            relative_path: PathBuf::from("skills/engineering/diagnose"),
        });

        assert_eq!(score_skill_choice("performance", &choice, "", 0), Some(10));
        assert_eq!(score_skill_choice("engineering", &choice, "", 0), Some(50));
        assert_eq!(score_skill_choice("diagnose", &choice, "", 0), Some(100));
        assert_eq!(score_skill_choice("missing", &choice, "", 0), None);
    }

    #[test]
    fn compact_revision_shortens_hashes_for_human_tables() {
        assert_eq!(
            compact_revision("0123456789abcdef0123456789abcdef01234567"),
            "0123456789ab"
        );
    }

    #[test]
    fn compact_source_keeps_start_and_end_visible() {
        let compact = compact_source("https://github.com/example/really-long-repository-name.git");

        assert!(compact.starts_with("https://github.com"));
        assert!(compact.ends_with("repository-name.git"));
        assert!(compact.chars().count() <= 42);
    }

    #[test]
    fn truncate_middle_handles_short_and_long_values() {
        assert_eq!(truncate_middle("short", 10), "short");
        assert_eq!(truncate_middle("abcdefghij", 7), "abc…hij");
    }

    #[test]
    fn list_state_labels_are_cli_friendly() {
        assert_eq!(
            list_state_label(&crate::application::list::ListedTargetState::SourceMissing),
            "source-missing"
        );
        assert_eq!(
            list_state_label(&crate::application::list::ListedTargetState::ResolveFailed(
                "bad target".to_owned()
            )),
            "resolve-failed"
        );
    }

    #[test]
    fn name_option_selects_matching_discovered_skill() {
        let selected = select_skill_candidates(
            "owner/repo",
            Some("review"),
            vec![
                super::SkillCandidate {
                    name: "find-skills".to_owned(),
                    description: "Find skills".to_owned(),
                    relative_path: PathBuf::from("skills/find-skills"),
                },
                super::SkillCandidate {
                    name: "review".to_owned(),
                    description: "Review helper".to_owned(),
                    relative_path: PathBuf::from("skills/review"),
                },
            ],
        )
        .unwrap();

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].relative_path, Path::new("skills/review"));
    }

    #[test]
    fn selected_subpath_is_appended_to_github_shorthand_source() {
        assert_eq!(
            source_with_selected_subpath(
                "vercel-labs/skills#main",
                Path::new("skills/find-skills"),
                SourceRewriteMode::Append,
            ),
            "vercel-labs/skills/skills/find-skills#main"
        );
    }

    #[test]
    fn selected_subpath_is_appended_to_github_url_as_tree_source() {
        assert_eq!(
            source_with_selected_subpath(
                "https://github.com/vercel-labs/skills",
                Path::new("skills/find-skills"),
                SourceRewriteMode::Append,
            ),
            "https://github.com/vercel-labs/skills/tree/HEAD/skills/find-skills"
        );
    }

    #[test]
    fn selected_subpath_preserves_github_url_reference_as_tree_ref() {
        assert_eq!(
            source_with_selected_subpath(
                "https://github.com/vercel-labs/skills#v1",
                Path::new("skills/find-skills"),
                SourceRewriteMode::Append,
            ),
            "https://github.com/vercel-labs/skills/tree/v1/skills/find-skills"
        );
    }

    #[test]
    fn selected_subpath_is_appended_to_skills_sh_source_as_skill_name() {
        assert_eq!(
            source_with_selected_subpath(
                "https://www.skills.sh/vercel-labs/skills",
                Path::new("skills/find-skills"),
                SourceRewriteMode::Append,
            ),
            "https://www.skills.sh/vercel-labs/skills/find-skills"
        );
    }

    #[test]
    fn selected_subpath_preserves_skills_sh_parent_path() {
        assert_eq!(
            source_with_selected_subpath(
                "skills.sh/owner/repo/category",
                Path::new("foo"),
                SourceRewriteMode::Append,
            ),
            "skills.sh/owner/repo/category/foo"
        );
    }

    #[test]
    fn selected_subpath_replaces_missing_skills_sh_direct_path() {
        assert_eq!(
            source_with_selected_subpath(
                "https://www.skills.sh/mattpocock/skills/grill-me",
                Path::new("skills/productivity/grill-me"),
                SourceRewriteMode::ReplaceSkillsShPath,
            ),
            "https://www.skills.sh/mattpocock/skills/productivity/grill-me"
        );
    }

    #[test]
    fn global_config_root_uses_home_dot_sksync() {
        assert_eq!(
            global_config_root_from_home(Path::new("/tmp/home")),
            Path::new("/tmp/home/.sksync")
        );
    }

    #[test]
    fn project_agent_mappings_override_global_mappings() {
        let mappings = agent_target_mappings_from_config(
            AgentMappingConfig {
                global: BTreeMap::from([("pi".to_owned(), PathBuf::from("~/.pi/skills"))]),
                project: BTreeMap::from([("pi".to_owned(), PathBuf::from(".pi/skills"))]),
            },
            Scope::Project,
        );

        assert_eq!(mappings["pi"].scope, Scope::Project);
        assert_eq!(mappings["pi"].target_dir, Path::new(".pi/skills"));
    }

    #[test]
    fn global_agent_mappings_ignore_project_mappings() {
        let mappings = agent_target_mappings_from_config(
            AgentMappingConfig {
                global: BTreeMap::from([("pi".to_owned(), PathBuf::from("~/.pi/skills"))]),
                project: BTreeMap::from([("pi".to_owned(), PathBuf::from(".pi/skills"))]),
            },
            Scope::User,
        );

        assert_eq!(mappings["pi"].scope, Scope::User);
        assert_eq!(mappings["pi"].target_dir, Path::new("~/.pi/skills"));
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
