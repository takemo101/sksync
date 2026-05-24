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
use crate::domain::removal::{classify_skill_removal, SkillRemovalScope};
use crate::domain::scope::Scope;
use crate::domain::skill::SkillName;
use crate::domain::skill_manifest::parse_skill_manifest;
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
    /// Attach an existing dependency-managed skill to more agents.
    Attach(AttachArgs),
    /// Inspect and manage agent target mappings.
    Agents(AgentsArgs),
    /// Diagnose config, lockfile, links, sources, and agent mappings without mutating files.
    Doctor(DoctorArgs),
    /// Import existing skill directories into sksync without touching originals.
    Import(ImportArgs),
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
struct AttachArgs {
    /// Existing dependency-managed skill name to attach.
    skill: String,
    /// Agent to link into. Can be passed multiple times.
    #[arg(short, long = "agent", required = true)]
    agents: Vec<String>,
    /// Use ~/.sksync/config.json instead of project config.
    #[arg(long)]
    global: bool,
}

#[derive(Debug, Args)]
struct AgentsArgs {
    #[command(subcommand)]
    command: AgentsCommand,
}

#[derive(Debug, Subcommand)]
enum AgentsCommand {
    /// List effective agent target mappings.
    List,
    /// Refresh ~/.sksync/agents.json from bundled mappings.
    Refresh,
    /// Diagnose agent target mappings without changing the filesystem.
    Doctor,
}

#[derive(Debug, Args)]
struct DoctorArgs {
    /// Use ~/.sksync/config.json instead of project config.
    #[arg(long)]
    global: bool,
}

#[derive(Debug, Args)]
struct ImportArgs {
    /// Existing directory containing one or more skills.
    path: PathBuf,
    /// Agent to attach imported skills to. Can be passed multiple times.
    #[arg(short, long = "agent", required = true)]
    agents: Vec<String>,
    /// Use ~/.sksync/config.json instead of project config.
    #[arg(long)]
    global: bool,
    /// Show what would be imported without writing files or config.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Args)]
struct RemoveArgs {
    /// Skill name(s) to remove.
    #[arg(required = true)]
    skills: Vec<String>,
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
        Command::Attach(args) => run_attach(args),
        Command::Agents(args) => run_agents(args),
        Command::Doctor(args) => run_doctor(args),
        Command::Import(args) => run_import(args),
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

fn run_agents(args: AgentsArgs) -> Result<()> {
    match args.command {
        AgentsCommand::List => run_agents_list(),
        AgentsCommand::Refresh => run_agents_refresh(),
        AgentsCommand::Doctor => run_agents_doctor(),
    }
}

fn run_agents_list() -> Result<()> {
    let mappings = merged_agent_mapping_config()?;
    print_agent_mapping_scope("Global agent mappings", &mappings.global);
    print_agent_mapping_scope("Project agent mappings", &mappings.project);
    Ok(())
}

fn run_agents_refresh() -> Result<()> {
    let result = init_agents(config_root_for_global()?)?;
    print_success(format!(
        "Updated agent mappings: {}",
        result.agent_mapping_path.display()
    ));
    Ok(())
}

fn run_agents_doctor() -> Result<()> {
    let diagnostics = collect_agent_diagnostics()?;
    print_agent_diagnostics(&diagnostics);
    if diagnostics.iter().any(AgentDiagnostic::is_error) {
        bail!("agents doctor found problem(s)");
    }
    Ok(())
}

fn run_doctor(args: DoctorArgs) -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let mut problems = Vec::new();
    let mut warnings = Vec::new();

    match load_config_for_scope(args.global, &current_dir) {
        Ok(config) => {
            let root_dir = if args.global {
                config_root_for_global()?
            } else {
                current_dir.clone()
            };
            let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
            let target_resolver = TargetPathResolver::new(&root_dir, home_dir);
            match build_desired_link_plan(&config, &target_resolver) {
                Ok(plan) => {
                    for item in &plan.items {
                        match &item.action {
                            PlanAction::CreateSymlink | PlanAction::AlreadySynced => {}
                            PlanAction::SourceMissing => problems.push(format!(
                                "{} -> {}: source missing; try `sksync update` or `sksync install`",
                                item.skill,
                                item.agent.as_str()
                            )),
                            PlanAction::Conflict { reason } => problems.push(format!(
                                "{} -> {}: target conflict ({reason}); inspect with `sksync plan`",
                                item.skill,
                                item.agent.as_str()
                            )),
                            PlanAction::DriftedSymlink { actual_source } => problems.push(format!(
                                "{} -> {}: symlink drifted to {}; inspect with `sksync plan` or re-run `sksync apply --force` if safe",
                                item.skill,
                                item.agent.as_str(),
                                actual_source.display()
                            )),
                        }
                    }

                    match read_lockfile(lockfile_path_for(args.global, &current_dir)?) {
                        Ok(lockfile) => {
                            let report = check_lockfile_with_plan(
                                &lockfile,
                                &plan,
                                &Sha256SourceHashStore,
                                &FileSystemLinkStore,
                            );
                            for problem in report.problems {
                                problems.push(format!(
                                    "lockfile/link health: {}; try `sksync install`, `sksync update`, or `sksync apply`",
                                    problem.display_line()
                                ));
                            }
                        }
                        Err(error) => problems.push(format!(
                            "lockfile: failed to load ({error}); try `sksync install`"
                        )),
                    }
                }
                Err(error) => problems.push(format!(
                    "plan: failed to build desired link plan ({error}); try `sksync plan`"
                )),
            }
        }
        Err(error) => problems.push(format!(
            "config: failed to load ({error}); try `sksync init{}`",
            if args.global { " --global" } else { "" }
        )),
    }

    for diagnostic in collect_agent_diagnostics()? {
        let message = format!(
            "agent mapping: {} {} -> {}: {}; try `sksync agents doctor`",
            diagnostic.scope,
            diagnostic.name,
            diagnostic.target.display(),
            diagnostic.status
        );
        if diagnostic.is_error() {
            problems.push(message);
        } else if diagnostic.is_warning() {
            warnings.push(message);
        }
    }

    if !warnings.is_empty() {
        print_section_with_count("Doctor warnings", warnings.len());
        for warning in &warnings {
            println!("! {warning}");
        }
    }

    if problems.is_empty() {
        if warnings.is_empty() {
            print_success(
                "Doctor passed. Config, lockfile, links, sources, and agent mappings look healthy.",
            );
        } else {
            print_success("Doctor passed with warning(s). No required repair was detected.");
        }
        return Ok(());
    }

    print_section_with_count("Doctor problems", problems.len());
    for problem in &problems {
        println!("✗ {problem}");
    }
    bail!("doctor found {} problem(s)", problems.len())
}

fn run_import(args: ImportArgs) -> Result<()> {
    parse_agent_kinds(&args.agents)?;
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let config_path = config_path_for(args.global, &current_dir)?;
    let config = if config_path.exists() {
        Some(
            load_config_for_scope(args.global, &current_dir)
                .context("failed to load config before import")?,
        )
    } else {
        None
    };
    let configured = config
        .as_ref()
        .map(|config| {
            config
                .skills
                .iter()
                .map(|skill| skill.name.as_str().to_owned())
                .collect::<std::collections::BTreeSet<_>>()
        })
        .unwrap_or_default();
    let skill_dir = config
        .map(|config| config.skill_dir.as_path().to_path_buf())
        .unwrap_or(resolve_default_skill_dir(args.global, &current_dir)?);
    let scan = scan_import_candidates(&args.path, &skill_dir, &configured)?;

    print_import_scan(&scan, args.dry_run);
    if args.dry_run {
        return Ok(());
    }
    if scan.importable.is_empty() {
        bail!("no importable skills found")
    }
    if !scan.conflicts.is_empty() {
        bail!("import has conflict(s); rerun with --dry-run for details")
    }

    let backup = ConfigFileBackup::capture(&config_path)?;
    let mut copied_destinations = Vec::new();
    let result = (|| -> Result<()> {
        let store =
            FileDependencyConfigStore::new(&config_path, default_skill_dir_for(args.global)?);
        for candidate in &scan.importable {
            if candidate.destination.exists() {
                bail!(
                    "destination already exists: {}",
                    candidate.destination.display()
                )
            }
            copied_destinations.push(candidate.destination.clone());
            copy_dir_all(&candidate.source, &candidate.destination)?;
            let source = config_source_for_path(&candidate.destination, &current_dir);
            store.add_dependency(&candidate.name, &source, &args.agents)?;
            print_success(format!(
                "Imported {} -> {}",
                candidate.name,
                candidate.destination.display()
            ));
        }
        Ok(())
    })();

    if let Err(error) = result {
        for destination in copied_destinations.iter().rev() {
            if destination.exists() {
                let _ = fs::remove_dir_all(destination);
            }
        }
        if let Err(restore_error) = backup.restore() {
            return Err(error.context(format!(
                "sksync import failed and config rollback failed: {restore_error}"
            )));
        }
        return Err(
            error.context("sksync import failed; restored previous config and copied files")
        );
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct AgentDiagnostic {
    scope: &'static str,
    name: String,
    target: PathBuf,
    status: String,
}

impl AgentDiagnostic {
    fn is_ok(&self) -> bool {
        self.status == "ok"
    }

    fn is_warning(&self) -> bool {
        self.status == "missing"
    }

    fn is_error(&self) -> bool {
        !self.is_ok() && !self.is_warning()
    }
}

#[derive(Debug, Clone)]
struct ImportCandidate {
    name: String,
    source: PathBuf,
    destination: PathBuf,
}

#[derive(Debug, Default)]
struct ImportScan {
    importable: Vec<ImportCandidate>,
    conflicts: Vec<ImportCandidate>,
    invalid: Vec<String>,
}

fn print_agent_mapping_scope(label: &str, mappings: &BTreeMap<String, PathBuf>) {
    print_section_with_count(label, mappings.len());
    if mappings.is_empty() {
        print_info("No mappings configured.");
        return;
    }
    let rows = mappings
        .iter()
        .map(|(agent, target)| vec![agent.clone(), target.display().to_string()])
        .collect::<Vec<_>>();
    print_table(&["Agent", "Target"], &rows);
}

fn collect_agent_diagnostics() -> Result<Vec<AgentDiagnostic>> {
    let mappings = merged_agent_mapping_config()?;
    let project_root = std::env::current_dir().context("failed to determine current directory")?;
    let global_root = config_root_for_global()?;
    let mut diagnostics = Vec::new();
    for (name, target) in mappings.global {
        diagnostics.push(agent_diagnostic("global", name, target, &global_root));
    }
    for (name, target) in mappings.project {
        diagnostics.push(agent_diagnostic("project", name, target, &project_root));
    }
    Ok(diagnostics)
}

fn agent_diagnostic(
    scope: &'static str,
    name: String,
    target: PathBuf,
    root: &Path,
) -> AgentDiagnostic {
    let resolved = resolve_agent_path(&target, root);
    let status = match fs::metadata(&resolved) {
        Ok(metadata) if !metadata.is_dir() => "not a directory".to_owned(),
        Ok(metadata) if metadata.permissions().readonly() => "read-only".to_owned(),
        Ok(_) => "ok".to_owned(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "missing".to_owned(),
        Err(error) => format!("unreadable: {error}"),
    };
    AgentDiagnostic {
        scope,
        name,
        target: resolved,
        status,
    }
}

fn resolve_agent_path(path: &Path, root: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if raw == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn print_agent_diagnostics(diagnostics: &[AgentDiagnostic]) {
    print_section_with_count("Agent diagnostics", diagnostics.len());
    for diagnostic in diagnostics {
        let badge = if diagnostic.is_ok() {
            "OK"
        } else if diagnostic.is_warning() {
            "WARN"
        } else {
            "ISSUE"
        };
        println!("{badge:<6} {} {}", diagnostic.scope, diagnostic.name);
        print_detail(format!("target: {}", diagnostic.target.display()));
        print_detail(format!("status: {}", diagnostic.status));
    }
}

fn resolve_default_skill_dir(global: bool, current_dir: &Path) -> Result<PathBuf> {
    let default = default_skill_dir_for(global)?;
    let global_root;
    let root = if global {
        global_root = config_root_for_global()?;
        global_root.as_path()
    } else {
        current_dir
    };
    Ok(resolve_agent_path(&default, root))
}

fn validate_import_skill_name(name: &str) -> Result<SkillName> {
    let skill_name = SkillName::new(name.to_owned())
        .with_context(|| format!("invalid skill name in SKILL.md: {name:?}"))?;
    if skill_name.as_str() == "." || skill_name.as_str() == ".." {
        bail!("invalid skill name in SKILL.md: {name:?}")
    }
    Ok(skill_name)
}

fn scan_import_candidates(
    root: &Path,
    skill_dir: &Path,
    configured: &std::collections::BTreeSet<String>,
) -> Result<ImportScan> {
    let mut scan = ImportScan::default();
    scan_import_candidates_inner(root, root, skill_dir, configured, 0, &mut scan)?;
    Ok(scan)
}

fn scan_import_candidates_inner(
    root: &Path,
    dir: &Path,
    skill_dir: &Path,
    configured: &std::collections::BTreeSet<String>,
    depth: usize,
    scan: &mut ImportScan,
) -> Result<()> {
    if depth > 5 {
        return Ok(());
    }
    let manifest_path = dir.join("SKILL.md");
    if manifest_path.exists() {
        match fs::read_to_string(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))
            .and_then(|content| parse_skill_manifest(&content).map_err(Into::into))
        {
            Ok(manifest) => {
                let skill_name = validate_import_skill_name(&manifest.name)?;
                let candidate = ImportCandidate {
                    name: skill_name.as_str().to_owned(),
                    source: dir.to_path_buf(),
                    destination: skill_dir.join(skill_name.as_str()),
                };
                if configured.contains(&candidate.name) || candidate.destination.exists() {
                    scan.conflicts.push(candidate);
                } else {
                    scan.importable.push(candidate);
                }
            }
            Err(error) => {
                let relative = dir.strip_prefix(root).unwrap_or(dir);
                scan.invalid
                    .push(format!("{}: {error}", relative.display()));
            }
        }
        return Ok(());
    }

    if !dir.is_dir() {
        bail!("import source is not a directory: {}", dir.display())
    }
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry =
            entry.with_context(|| format!("failed to read entry under {}", dir.display()))?;
        if entry.file_type()?.is_dir() {
            scan_import_candidates_inner(
                root,
                &entry.path(),
                skill_dir,
                configured,
                depth + 1,
                scan,
            )?;
        }
    }
    Ok(())
}

fn print_import_scan(scan: &ImportScan, dry_run: bool) {
    let label = if dry_run {
        "Import dry run"
    } else {
        "Import plan"
    };
    print_section_with_count(label, scan.importable.len() + scan.conflicts.len());
    for candidate in &scan.importable {
        println!("IMPORT {}", candidate.name);
        print_detail(format!("from: {}", candidate.source.display()));
        print_detail(format!("to: {}", candidate.destination.display()));
    }
    for candidate in &scan.conflicts {
        println!("CONFLICT {}", candidate.name);
        print_detail(format!("from: {}", candidate.source.display()));
        print_detail(format!("to: {}", candidate.destination.display()));
    }
    for invalid in &scan.invalid {
        println!("INVALID {invalid}");
    }
}

fn config_source_for_path(path: &Path, current_dir: &Path) -> String {
    if let Ok(relative) = path.strip_prefix(current_dir) {
        return format!("./{}", relative.display());
    }
    path.display().to_string()
}

fn copy_dir_all(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        bail!("destination already exists: {}", destination.display())
    }
    fs::create_dir_all(destination)
        .with_context(|| format!("failed to create {}", destination.display()))?;
    let result = copy_dir_contents(source, destination);
    if result.is_err() && destination.exists() {
        let _ = fs::remove_dir_all(destination);
    }
    result
}

fn copy_dir_contents(source: &Path, destination: &Path) -> Result<()> {
    for entry in
        fs::read_dir(source).with_context(|| format!("failed to read {}", source.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry under {}", source.display()))?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    entry.path().display(),
                    target.display()
                )
            })?;
        }
    }
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

fn run_attach(args: AttachArgs) -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let config_path = config_path_for(args.global, &current_dir)?;
    let config_backup = ConfigFileBackup::capture(&config_path)?;
    let attach_result = (|| -> Result<()> {
        let store =
            FileDependencyConfigStore::new(&config_path, default_skill_dir_for(args.global)?);
        let agents = store.add_dependency_agents(&args.skill, &args.agents)?;
        let mut config = load_config_from_path(&config_path, scope_for(args.global))?;
        let update_report = update_dependencies(&config, &FileSystemSkillInstaller)?;
        apply_update_report_sources(&mut config, &update_report);
        let fs_store = FileSystemLinkStore;
        let root_dir = if args.global {
            config_root_for_global()?
        } else {
            current_dir.clone()
        };
        let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
        let target_resolver = TargetPathResolver::new(&root_dir, home_dir);
        let plan = build_link_plan(&config, &fs_store, &fs_store, &target_resolver)?;
        let lockfile = build_lockfile_from_plan(&config, &plan, &root_dir)?;
        apply_link_plan(
            &plan,
            &lockfile,
            &fs_store,
            &FileLockfileStore::new(lockfile_path_for(args.global, &current_dir)?),
            ApplyOptions {
                force: false,
                skip_blocked_targets: true,
            },
        )?;
        print_success(format!(
            "Attached dependency: {} -> {}",
            args.skill,
            agents.join(", ")
        ));
        print_update_report(update_report);
        print_plan(&plan);
        Ok(())
    })();

    if let Err(error) = attach_result {
        if let Err(restore_error) = config_backup.restore() {
            return Err(error.context(format!(
                "sksync attach failed and config rollback failed: {restore_error}"
            )));
        }
        return Err(error.context("sksync attach failed; restored previous config"));
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
    let lockfile_path = lockfile_path_for(args.global, &current_dir)?;
    let config_backup = ConfigFileBackup::capture(&config_path)?;
    let lockfile_backup = ConfigFileBackup::capture(&lockfile_path)?;

    let remove_result = (|| -> Result<()> {
        let config = load_config_for_scope(args.global, &current_dir)?;
        let skill_dir = config.skill_dir.as_path().to_path_buf();
        let mut lockfile = read_lockfile(&lockfile_path).ok();
        let (_config, removal_plan, _root_dir) =
            build_plan_from_config(config.clone(), args.global, &current_dir)?;
        let runtime = RemoveRuntime {
            config_path: &config_path,
            skill_dir: &skill_dir,
            lockfile_path: &lockfile_path,
            removal_plan: &removal_plan,
        };
        let requested_agents = parse_agent_kinds(&args.agents)?;

        for skill_name in &args.skills {
            let skill = config
                .skills
                .iter()
                .find(|skill| skill.name.as_str() == skill_name);
            let skill_source = skill.map(|skill| skill.source.as_path().to_path_buf());
            match classify_skill_removal(
                skill.map(|skill| skill.agents.as_slice()).unwrap_or(&[]),
                &requested_agents,
            ) {
                SkillRemovalScope::EntireSkill => {
                    remove_entire_skill(&args, skill_name, skill_source, &mut lockfile, &runtime)?
                }
                SkillRemovalScope::SelectedAgents => remove_skill_agents(
                    &args,
                    skill_name,
                    &mut lockfile,
                    &requested_agents,
                    &runtime,
                )?,
            }
        }
        Ok(())
    })();

    if let Err(error) = remove_result {
        let config_restore = config_backup.restore();
        let lockfile_restore = lockfile_backup.restore();
        if let Err(restore_error) = config_restore.and(lockfile_restore) {
            return Err(error.context(format!(
                "sksync remove failed and rollback failed: {restore_error}"
            )));
        }
        return Err(error.context("sksync remove failed; restored previous config and lockfile"));
    }

    Ok(())
}

struct RemoveRuntime<'a> {
    config_path: &'a Path,
    skill_dir: &'a Path,
    lockfile_path: &'a Path,
    removal_plan: &'a LinkPlan,
}

fn remove_entire_skill(
    args: &RemoveArgs,
    skill: &str,
    skill_source: Option<PathBuf>,
    lockfile: &mut Option<Lockfile>,
    runtime: &RemoveRuntime<'_>,
) -> Result<()> {
    if !args.config_only {
        remove_managed_symlinks(runtime.removal_plan, skill)?;
        if !args.keep_files {
            if let Some(source) = skill_source {
                remove_installed_skill_dir(&source, runtime.skill_dir)?;
            }
        }
    }

    FileDependencyConfigStore::new(runtime.config_path, default_skill_dir_for(args.global)?)
        .remove_dependency(skill)?;
    if let Some(lockfile) = lockfile {
        if let Ok(skill_name) = crate::domain::skill::SkillName::new(skill.to_owned()) {
            lockfile.skills.remove(&skill_name);
            FileLockfileStore::new(runtime.lockfile_path).write(lockfile)?;
        }
    }
    print_success(format!("Removed skill: {skill}"));
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
    skill: &str,
    lockfile: &mut Option<Lockfile>,
    requested_agents: &[AgentKind],
    runtime: &RemoveRuntime<'_>,
) -> Result<()> {
    if !args.config_only {
        remove_managed_symlinks_for_agents(runtime.removal_plan, skill, requested_agents)?;
    }

    let requested_agent_names = requested_agents
        .iter()
        .map(|agent| agent.as_str().to_owned())
        .collect::<Vec<_>>();
    let remaining_agents =
        FileDependencyConfigStore::new(runtime.config_path, default_skill_dir_for(args.global)?)
            .remove_dependency_agents(skill, &requested_agent_names)?;

    if let Some(lockfile) = lockfile {
        if let Ok(skill_name) = crate::domain::skill::SkillName::new(skill.to_owned()) {
            if let Some(locked) = lockfile.skills.get_mut(&skill_name) {
                locked
                    .targets
                    .retain(|target| !agent_kinds_contain(requested_agents, &target.agent));
            }
            FileLockfileStore::new(runtime.lockfile_path).write(lockfile)?;
        }
    }

    print_success(format!(
        "Detached {skill} from agent(s): {}",
        requested_agent_names.join(", ")
    ));
    if remaining_agents.is_empty() {
        print_info(format!(
            "No agents remain for {skill}; removed dependency entry"
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
        ApplyOptions {
            force: args.force,
            skip_blocked_targets: false,
        },
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
        ApplyOptions {
            force: false,
            skip_blocked_targets: false,
        },
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
        agent_target_mappings_from_config, compact_revision, compact_source, copy_dir_all,
        format_selected_skill_choices, global_config_root_from_home, is_managed_skill_dir,
        list_state_label, reject_legacy_registry_source, remove_installed_skill_dir,
        scan_import_candidates, score_skill_choice, select_skill_candidates, truncate_middle, Cli,
        Command, ConfigFileBackup,
    };
    use crate::application::discovery::{
        discover_skill_candidates, source_with_selected_subpath, SourceRewriteMode,
    };
    use crate::domain::scope::Scope;
    use crate::infrastructure::json::AgentMappingConfig;
    use clap::{CommandFactory, Parser};
    use std::collections::{BTreeMap, BTreeSet};
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
                "init", "add", "attach", "agents", "doctor", "import", "remove", "outdated",
                "plan", "apply", "install", "update", "check", "list", "wizard",
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
    fn agents_subcommands_are_registered() {
        Cli::try_parse_from(["sksync", "agents", "list"]).expect("agents list parses");
        Cli::try_parse_from(["sksync", "agents", "refresh"]).expect("agents refresh parses");
        Cli::try_parse_from(["sksync", "agents", "doctor"]).expect("agents doctor parses");
    }

    #[test]
    fn doctor_command_is_registered() {
        Cli::try_parse_from(["sksync", "doctor"]).expect("doctor parses");
        Cli::try_parse_from(["sksync", "doctor", "--global"]).expect("doctor --global parses");
    }

    #[test]
    fn import_requires_agent_and_accepts_dry_run() {
        assert!(Cli::try_parse_from(["sksync", "import", "./skills"]).is_err());
        Cli::try_parse_from([
            "sksync",
            "import",
            "./skills",
            "--agent",
            "jcode",
            "--dry-run",
        ])
        .expect("import --agent --dry-run parses");
    }

    #[test]
    fn import_scan_detects_valid_skills_and_conflicts() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("agent-skills");
        let skill_dir = temp.path().join(".sksync/skills");
        fs::create_dir_all(source.join("review")).expect("create review");
        fs::create_dir_all(source.join("existing")).expect("create existing");
        fs::write(
            source.join("review/SKILL.md"),
            "---\nname: review\ndescription: Review code\n---\n# Review\n",
        )
        .expect("write review manifest");
        fs::write(
            source.join("existing/SKILL.md"),
            "---\nname: existing\ndescription: Existing skill\n---\n# Existing\n",
        )
        .expect("write existing manifest");
        fs::create_dir_all(skill_dir.join("existing")).expect("create conflict destination");

        let configured = BTreeSet::new();
        let scan = scan_import_candidates(&source, &skill_dir, &configured).expect("scan import");

        assert_eq!(
            scan.importable
                .iter()
                .map(|candidate| candidate.name.as_str())
                .collect::<Vec<_>>(),
            vec!["review"]
        );
        assert_eq!(
            scan.conflicts
                .iter()
                .map(|candidate| candidate.name.as_str())
                .collect::<Vec<_>>(),
            vec!["existing"]
        );
    }

    #[test]
    fn import_scan_rejects_skill_names_that_escape_skill_dir() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("agent-skills");
        let skill_dir = temp.path().join(".sksync/skills");
        fs::create_dir_all(source.join("escaped")).expect("create escaped");
        fs::write(
            source.join("escaped/SKILL.md"),
            "---\nname: ..\ndescription: Escape\n---\n# Escape\n",
        )
        .expect("write escaping manifest");

        let configured = BTreeSet::new();
        let error = scan_import_candidates(&source, &skill_dir, &configured)
            .expect_err("escaping skill name must fail");

        assert!(error.to_string().contains("invalid skill name"));
        assert!(!skill_dir.exists());
    }

    #[test]
    fn import_scan_rejects_skill_names_with_separators() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("agent-skills");
        let skill_dir = temp.path().join(".sksync/skills");
        fs::create_dir_all(source.join("nested")).expect("create nested");
        fs::write(
            source.join("nested/SKILL.md"),
            "---\nname: foo/bar\ndescription: Nested\n---\n# Nested\n",
        )
        .expect("write nested manifest");

        let configured = BTreeSet::new();
        let error = scan_import_candidates(&source, &skill_dir, &configured)
            .expect_err("separator skill name must fail");

        assert!(error.to_string().contains("invalid skill name"));
        assert!(!skill_dir.exists());
    }

    #[test]
    fn import_scan_reports_invalid_skill_manifests_without_writing() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("agent-skills");
        let skill_dir = temp.path().join(".sksync/skills");
        fs::create_dir_all(source.join("broken")).expect("create broken");
        fs::write(source.join("broken/SKILL.md"), "# Missing frontmatter\n")
            .expect("write broken manifest");

        let configured = BTreeSet::new();
        let scan = scan_import_candidates(&source, &skill_dir, &configured).expect("scan import");

        assert!(scan.importable.is_empty());
        assert!(scan.conflicts.is_empty());
        assert_eq!(scan.invalid.len(), 1);
        assert!(!skill_dir.exists());
    }

    #[test]
    fn copy_dir_all_cleans_partial_destination_on_copy_failure() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir_all(&source).expect("create source");
        fs::write(source.join("copied-first.txt"), "partial").expect("write regular file");
        std::os::unix::fs::symlink(source.join("missing"), source.join("dangling"))
            .expect("create dangling symlink");

        let error = copy_dir_all(&source, &destination).expect_err("dangling symlink should fail");

        assert!(error.to_string().contains("failed to copy"));
        assert!(!destination.exists());
    }

    #[test]
    fn attach_requires_agent() {
        assert!(Cli::try_parse_from(["sksync", "attach", "review"]).is_err());
        Cli::try_parse_from(["sksync", "attach", "review", "--agent", "pi"])
            .expect("attach --agent should parse");
    }

    #[test]
    fn remove_accepts_multiple_skills() {
        let cli =
            Cli::try_parse_from(["sksync", "remove", "one", "two"]).expect("remove should parse");
        let Command::Remove(args) = cli.command else {
            panic!("expected remove command");
        };

        assert_eq!(args.skills, vec!["one", "two"]);
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
    fn selected_subpath_from_skills_sh_repo_root_becomes_github_tree_url() {
        assert_eq!(
            source_with_selected_subpath(
                "https://www.skills.sh/vercel-labs/skills",
                Path::new("skills/find-skills"),
                SourceRewriteMode::Append,
            ),
            "https://github.com/vercel-labs/skills/tree/HEAD/skills/find-skills"
        );
    }

    #[test]
    fn selected_subpath_from_skills_sh_parent_path_becomes_github_tree_url() {
        assert_eq!(
            source_with_selected_subpath(
                "skills.sh/owner/repo/category",
                Path::new("foo"),
                SourceRewriteMode::Append,
            ),
            "https://github.com/owner/repo/tree/HEAD/skills/category/foo"
        );
    }

    #[test]
    fn selected_subpath_from_skills_sh_direct_path_becomes_github_tree_url() {
        assert_eq!(
            source_with_selected_subpath(
                "https://www.skills.sh/mattpocock/skills/grill-me",
                Path::new("skills/productivity/grill-me"),
                SourceRewriteMode::ReplaceSkillsShPath,
            ),
            "https://github.com/mattpocock/skills/tree/HEAD/skills/productivity/grill-me"
        );
    }

    #[test]
    fn direct_skills_sh_selected_subpath_becomes_github_tree_url() {
        assert_eq!(
            source_with_selected_subpath(
                "https://www.skills.sh/vercel-labs/skills/find-skills#main",
                Path::new("."),
                SourceRewriteMode::Append,
            ),
            "https://github.com/vercel-labs/skills/tree/main/skills/find-skills"
        );
    }

    #[test]
    fn selected_subpath_outside_skills_sh_skills_dir_becomes_github_tree_url() {
        assert_eq!(
            source_with_selected_subpath(
                "https://www.skills.sh/gitbutlerapp/gitbutler/but",
                Path::new("crates/but/skill"),
                SourceRewriteMode::ReplaceSkillsShPath,
            ),
            "https://github.com/gitbutlerapp/gitbutler/tree/HEAD/crates/but/skill"
        );
    }

    #[test]
    fn selected_subpath_outside_skills_sh_skills_dir_preserves_reference() {
        assert_eq!(
            source_with_selected_subpath(
                "skills.sh/gitbutlerapp/gitbutler/but#master",
                Path::new("crates/but/skill"),
                SourceRewriteMode::ReplaceSkillsShPath,
            ),
            "https://github.com/gitbutlerapp/gitbutler/tree/master/crates/but/skill"
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
