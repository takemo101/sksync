use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command as GitCommand;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::application::add::{run_add_workflow, AddSelection, AddWorkflow};
use crate::application::apply::{apply_link_plan, ApplyOptions};
use crate::application::bundle::{
    apply_bundle_export_plan, build_bundle_export_plan, discover_bundle_manifest_candidates,
    load_bundle_from_source, validate_bundle_export_plan, BundleAddPlan, BundleAddPlanItem,
    BundleAddStatus, BundleExportApplyOptions, BundleExportDestination, BundleExportMode,
    BundleExportPlan, BundleExportPlanInput, BundleExportResolvedSkill, BundleManifestCandidate,
    BundleRemovePlan, BundleRemovePlanItem, BundleRemoveStatus, BundleSyncPlan,
    BundleSyncSourceResolution, BundleSyncStatus, BUNDLE_MANIFEST_FILE,
};
use crate::application::check::{
    check_lockfile_with_config_and_plan, group_check_problems, CheckProblem, ProblemGroup,
};
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
use crate::application::ports::{AddDependencyOptions, DependencyConfigStore, LockfileStore};
use crate::application::remote::{
    collect_remote_source_problems, RemoteSourceChecker, RemoteSourceProblem, RemoteSourceStatus,
};
use crate::application::update::{apply_update_report_sources, update_dependencies};
use crate::domain::agent::AgentKind;
use crate::domain::bundle::BundleName;
use crate::domain::link_plan::{LinkPlan, LinkPlanItem, PlanAction};
use crate::domain::lockfile::{LockedFile, LockedSkill, Lockfile};
use crate::domain::package_filter::PackageFilter;
use crate::domain::removal::{classify_skill_removal, SkillRemovalScope};
use crate::domain::scope::Scope;
use crate::domain::skill::SkillName;
use crate::domain::skill_manifest::parse_skill_manifest;
use crate::domain::source::GitInstallSource;
use crate::infrastructure::builtin_agents::TargetPathResolver;
use crate::infrastructure::fs::FileSystemLinkStore;
use crate::infrastructure::git::GitClient;
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
    /// Inspect, add, and remove curated bundle install sets.
    Bundle(BundleArgs),
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
    #[arg(short = 'g', long)]
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
    /// Copy only matched files/directories from the resolved skill package root. Repeatable.
    #[arg(
        long = "include",
        value_name = "pattern",
        conflicts_with = "manifest_only"
    )]
    include: Vec<String>,
    /// Shortcut for --include SKILL.md.
    #[arg(long, conflicts_with = "include")]
    manifest_only: bool,
    /// Write ~/.sksync/config.json instead of ./sksync.config.json.
    #[arg(short = 'g', long)]
    global: bool,
    /// Replace drifted or broken target symlinks during the final link apply step.
    #[arg(short = 'f', long)]
    force: bool,
}

#[derive(Debug, Args)]
struct AttachArgs {
    /// Existing dependency-managed skill name to attach.
    skill: String,
    /// Agent to link into. Can be passed multiple times.
    #[arg(short, long = "agent", required = true)]
    agents: Vec<String>,
    /// Use ~/.sksync/config.json instead of project config.
    #[arg(short = 'g', long)]
    global: bool,
    /// Replace drifted or broken target symlinks during the final link apply step.
    #[arg(short = 'f', long)]
    force: bool,
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
    #[arg(short = 'g', long)]
    global: bool,
    /// Additionally check remote dependency source availability (network/git). Off by default.
    #[arg(long)]
    remote: bool,
}

#[derive(Debug, Args)]
struct ImportArgs {
    /// Existing directory containing one or more skills.
    path: PathBuf,
    /// Agent to attach imported skills to. Can be passed multiple times.
    #[arg(short, long = "agent", required = true)]
    agents: Vec<String>,
    /// Use ~/.sksync/config.json instead of project config.
    #[arg(short = 'g', long)]
    global: bool,
    /// Show what would be imported without writing files or config.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Args)]
struct BundleArgs {
    #[command(subcommand)]
    command: BundleCommand,
}

#[derive(Debug, Subcommand)]
enum BundleCommand {
    /// Inspect a bundle manifest without changing config or files.
    Inspect(BundleInspectArgs),
    /// Add all entries from a bundle to selected agents.
    Add(BundleAddArgs),
    /// Remove local dependency provenance for a bundle.
    Remove(BundleRemoveArgs),
    /// Synchronize local bundle provenance with the latest bundle manifest.
    Sync(BundleSyncArgs),
    /// Export current dependencies as a bundle manifest.
    Export(BundleExportArgs),
}

#[derive(Debug, Args)]
struct BundleInspectArgs {
    /// Bundle source directory, repo, or sksync.bundle.json file.
    source: String,
    /// Select one discovered bundle by manifest name or manifest parent directory name.
    #[arg(long)]
    name: Option<String>,
}

#[derive(Debug, Args)]
struct BundleAddArgs {
    /// Bundle source directory, repo, or sksync.bundle.json file.
    source: String,
    /// Select one discovered bundle by manifest name or manifest parent directory name.
    #[arg(long)]
    name: Option<String>,
    /// Agent to link bundle entries into. Can be passed multiple times.
    #[arg(short, long = "agent", required = true)]
    agents: Vec<String>,
    /// Use ~/.sksync/config.json instead of project config.
    #[arg(short = 'g', long)]
    global: bool,
    /// Show what would change without writing files or config.
    #[arg(long)]
    dry_run: bool,
    /// Replace drifted or broken target symlinks during the final link apply step.
    #[arg(short = 'f', long)]
    force: bool,
}

#[derive(Debug, Args)]
struct BundleRemoveArgs {
    /// Bundle name to remove from local provenance.
    name: String,
    /// Exact stored bundle source to disambiguate duplicate bundle names.
    #[arg(long)]
    source: Option<String>,
    /// Use ~/.sksync/config.json instead of project config.
    #[arg(short = 'g', long)]
    global: bool,
    /// Show what would change without writing files or config.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Args)]
struct BundleSyncArgs {
    /// Bundle name to synchronize from local provenance.
    name: String,
    /// Exact stored bundle source to disambiguate duplicate bundle names.
    #[arg(long)]
    source: Option<String>,
    /// Agent fallback for new entries when dependency agents cannot be inferred.
    #[arg(short, long = "agent")]
    agents: Vec<String>,
    /// Use ~/.sksync/config.json instead of project config.
    #[arg(short = 'g', long)]
    global: bool,
    /// Show what would change without writing files or config.
    #[arg(long)]
    dry_run: bool,
    /// Replace drifted or broken target symlinks during the final link apply step.
    #[arg(short = 'f', long)]
    force: bool,
}

#[derive(Debug, Args)]
struct BundleExportArgs {
    /// Bundle name to write into sksync.bundle.json.
    name: String,
    /// Output directory that will contain sksync.bundle.json.
    #[arg(long, required_unless_present = "root", conflicts_with = "root")]
    output: Option<PathBuf>,
    /// Write only sksync.bundle.json into the active configuration root.
    #[arg(long, conflicts_with_all = ["output", "snapshot"])]
    root: bool,
    /// Export from ~/.sksync/config.json instead of project config.
    #[arg(short = 'g', long)]
    global: bool,
    /// Copy currently installed skill bodies into the bundle directory.
    #[arg(long)]
    snapshot: bool,
    /// Export only selected dependency names. Repeatable.
    #[arg(long = "skill")]
    skills: Vec<String>,
    /// Show the export plan without writing files.
    #[arg(long)]
    dry_run: bool,
    /// Replace an existing generated output directory or root manifest.
    #[arg(short = 'f', long)]
    force: bool,
}

#[derive(Debug, Args)]
struct RemoveArgs {
    /// Skill name(s) to remove.
    #[arg(required = true)]
    skills: Vec<String>,
    /// Use ~/.sksync/config.json instead of project config.
    #[arg(short = 'g', long)]
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
    #[arg(short = 'g', long)]
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
    #[arg(short = 'g', long)]
    global: bool,
}

#[derive(Debug, Args)]
struct ApplyArgs {
    /// Replace drifted or broken target symlinks without touching files or directories.
    #[arg(short = 'f', long)]
    force: bool,
    /// Use ~/.sksync/config.json instead of project config.
    #[arg(short = 'g', long)]
    global: bool,
}

#[derive(Debug, Args)]
struct InstallArgs {
    /// Use ~/.sksync/config.json and global lockfile instead of project files.
    #[arg(short = 'g', long)]
    global: bool,
    /// Replace drifted or broken target symlinks during the final link apply step.
    #[arg(short = 'f', long)]
    force: bool,
}

#[derive(Debug, Args)]
struct UpdateArgs {
    /// Use ~/.sksync/config.json instead of project config.
    #[arg(short = 'g', long)]
    global: bool,
}

#[derive(Debug, Args)]
struct ListArgs {
    /// Use ~/.sksync/config.json instead of project config.
    #[arg(short = 'g', long)]
    global: bool,
}

#[derive(Debug, Args)]
struct CheckArgs {
    /// Use ~/.sksync/sksync-lock.json instead of project lockfile.
    #[arg(short = 'g', long)]
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
        Command::Bundle(args) => run_bundle(args),
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

/// Plan-level problems detected while building the desired link plan.
///
/// These come from comparing the desired plan against the current filesystem and are
/// reported separately from lockfile/link (`CheckProblem`) findings.
#[derive(Debug, Default)]
struct PlanProblems {
    /// `(skill, agent)` pairs whose source could not be found.
    source_missing: Vec<(String, String)>,
    /// `(skill, agent, reason)` for targets blocked by an existing path.
    conflicts: Vec<(String, String, String)>,
    /// `(skill, agent, actual_source)` for symlinks pointing somewhere unexpected.
    drift: Vec<(String, String, String)>,
}

impl PlanProblems {
    fn len(&self) -> usize {
        self.source_missing.len() + self.conflicts.len() + self.drift.len()
    }
}

fn run_doctor(args: DoctorArgs) -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let mut load_errors: Vec<String> = Vec::new();
    let mut plan_problems = PlanProblems::default();
    let mut check_problems: Vec<CheckProblem> = Vec::new();

    let config_result = load_config_for_scope(args.global, &current_dir);
    match &config_result {
        Ok(config) => {
            let root_dir = if args.global {
                config_root_for_global()?
            } else {
                current_dir.clone()
            };
            let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
            let target_resolver = TargetPathResolver::new(&root_dir, home_dir);
            match build_desired_link_plan(config, &target_resolver) {
                Ok(plan) => {
                    for item in &plan.items {
                        match &item.action {
                            PlanAction::CreateSymlink | PlanAction::AlreadySynced => {}
                            PlanAction::SourceMissing => plan_problems
                                .source_missing
                                .push((item.skill_label(), item.agent_label())),
                            PlanAction::Conflict { reason } => plan_problems.conflicts.push((
                                item.skill_label(),
                                item.agent_label(),
                                reason.to_string(),
                            )),
                            PlanAction::DriftedSymlink { actual_source } => {
                                plan_problems.drift.push((
                                    item.skill_label(),
                                    item.agent_label(),
                                    actual_source.display().to_string(),
                                ))
                            }
                        }
                    }

                    match read_lockfile(lockfile_path_for(args.global, &current_dir)?) {
                        Ok(lockfile) => {
                            let report = check_lockfile_with_config_and_plan(
                                config,
                                &lockfile,
                                &plan,
                                &Sha256SourceHashStore,
                                &FileSystemLinkStore,
                            );
                            check_problems.extend(report.problems);
                        }
                        Err(error) => load_errors.push(format!(
                            "lockfile: failed to load ({error}); try `sksync install`"
                        )),
                    }
                }
                Err(error) => load_errors.push(format!(
                    "plan: failed to build desired link plan ({error}); try `sksync plan`"
                )),
            }
        }
        Err(error) => load_errors.push(format!(
            "config: failed to load ({error}); try `sksync init{}`",
            if args.global { " --global" } else { "" }
        )),
    }

    let mut agent_problem_lines: Vec<String> = Vec::new();
    let mut agent_warning_lines: Vec<String> = Vec::new();
    for diagnostic in collect_agent_diagnostics()? {
        let line = format!(
            "{} · {} · {}: {}",
            diagnostic.scope,
            diagnostic.name,
            diagnostic.target.display(),
            diagnostic.status
        );
        if diagnostic.is_error() {
            agent_problem_lines.push(line);
        } else if diagnostic.is_warning() {
            agent_warning_lines.push(line);
        }
    }

    if args.remote && config_result.is_ok() {
        print_progress("Checking remote dependency sources...");
    }
    let remote_problems = doctor_remote_problems(
        &config_result,
        args.remote,
        args.global,
        &GitRemoteSourceChecker,
    );

    let local_groups = build_local_problem_groups(&load_errors, &plan_problems, &check_problems);
    let agent_problem_group = agent_mapping_group("Agent mapping problems", &agent_problem_lines);
    let agent_warning_group = agent_mapping_group("Agent mapping warnings", &agent_warning_lines);

    let sections = build_doctor_sections(
        &remote_problems,
        local_groups,
        agent_problem_group,
        agent_warning_group,
    );
    for section in &sections {
        match section {
            DoctorSection::Remote(problems) => print_remote_source_problems(problems),
            DoctorSection::Group(group) => print_problem_group(group),
        }
    }

    let total = remote_problems.len()
        + check_problems.len()
        + plan_problems.len()
        + load_errors.len()
        + agent_problem_lines.len();
    if total == 0 {
        if agent_warning_lines.is_empty() {
            print_success(
                "Doctor passed. Config, lockfile, links, sources, and agent mappings look healthy.",
            );
        } else {
            print_success("Doctor passed with warning(s). No required repair was detected.");
        }
        return Ok(());
    }

    bail!("doctor found {total} problem(s)")
}

/// A renderable section of `doctor` output, in print order.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DoctorSection {
    /// Detailed remote dependency-source problems.
    Remote(Vec<RemoteSourceProblem>),
    /// A grouped local/link or agent-mapping section sharing one fix hint.
    Group(ProblemGroup),
}

#[cfg(test)]
impl DoctorSection {
    fn title(&self) -> &str {
        match self {
            DoctorSection::Remote(_) => "Remote source problems",
            DoctorSection::Group(group) => &group.title,
        }
    }
}

/// Order doctor sections for printing.
///
/// Remote source problems are printed first so the purpose of `--remote` is not buried
/// under local/link problems. Agent-mapping problems and warnings are kept as their own
/// sections, separate from local/link groups.
fn build_doctor_sections(
    remote: &[RemoteSourceProblem],
    local_groups: Vec<ProblemGroup>,
    agent_problem_group: Option<ProblemGroup>,
    agent_warning_group: Option<ProblemGroup>,
) -> Vec<DoctorSection> {
    let mut sections = Vec::new();
    if !remote.is_empty() {
        sections.push(DoctorSection::Remote(remote.to_vec()));
    }
    sections.extend(local_groups.into_iter().map(DoctorSection::Group));
    sections.extend(agent_problem_group.map(DoctorSection::Group));
    sections.extend(agent_warning_group.map(DoctorSection::Group));
    sections
}

/// Build grouped local/link sections: setup errors, lockfile/link checks, then plan-level findings.
fn build_local_problem_groups(
    load_errors: &[String],
    plan: &PlanProblems,
    check_problems: &[CheckProblem],
) -> Vec<ProblemGroup> {
    let mut groups = Vec::new();

    if !load_errors.is_empty() {
        groups.push(ProblemGroup {
            title: "Setup".to_owned(),
            count: load_errors.len(),
            hint: String::new(),
            lines: load_errors.to_vec(),
        });
    }

    groups.extend(group_check_problems(check_problems));

    if !plan.source_missing.is_empty() {
        groups.push(ProblemGroup {
            title: "Missing sources".to_owned(),
            count: plan.source_missing.len(),
            hint: "run `sksync update` or `sksync install` to restore the source".to_owned(),
            lines: plan
                .source_missing
                .iter()
                .map(|(skill, agent)| format!("{skill} · {agent}"))
                .collect(),
        });
    }

    if !plan.conflicts.is_empty() {
        groups.push(ProblemGroup {
            title: "Plan conflicts".to_owned(),
            count: plan.conflicts.len(),
            hint: "inspect with `sksync plan`; remove or relocate the conflicting path(s)"
                .to_owned(),
            lines: plan
                .conflicts
                .iter()
                .map(|(skill, agent, reason)| format!("{skill} · {agent} ({reason})"))
                .collect(),
        });
    }

    if !plan.drift.is_empty() {
        groups.push(ProblemGroup {
            title: "Plan drift".to_owned(),
            count: plan.drift.len(),
            hint: "inspect with `sksync plan`, then `sksync apply --force` if the drift is safe to overwrite"
                .to_owned(),
            lines: plan
                .drift
                .iter()
                .map(|(skill, agent, actual)| format!("{skill} · {agent} → {actual}"))
                .collect(),
        });
    }

    groups
}

/// Build an agent-mapping section, keeping `sksync agents doctor` as fix guidance.
fn agent_mapping_group(title: &str, lines: &[String]) -> Option<ProblemGroup> {
    if lines.is_empty() {
        return None;
    }
    Some(ProblemGroup {
        title: title.to_owned(),
        count: lines.len(),
        hint: "run `sksync agents doctor`".to_owned(),
        lines: lines.to_vec(),
    })
}

fn print_problem_group(group: &ProblemGroup) {
    print_section_with_count(&group.title, group.count);
    if !group.hint.is_empty() {
        print_detail(format!("fix: {}", group.hint));
    }
    for line in &group.lines {
        print_detail(line);
    }
}

/// Collect remote dependency-source problems for `doctor`, honoring the `--remote` gate.
///
/// Returns an empty list and never invokes `checker` unless `remote` is set and the
/// config loaded successfully. This is the seam that keeps the default `doctor` run
/// local-only with no remote git operations.
fn doctor_remote_problems(
    config_result: &Result<ResolvedConfig>,
    remote: bool,
    global: bool,
    checker: &impl RemoteSourceChecker,
) -> Vec<RemoteSourceProblem> {
    if !remote {
        return Vec::new();
    }
    match config_result {
        Ok(config) => collect_remote_source_problems(config, global, checker).problems,
        Err(_) => Vec::new(),
    }
}

fn print_remote_source_problems(problems: &[RemoteSourceProblem]) {
    print_section_with_count("Remote source problems", problems.len());
    for problem in problems {
        println!("{}: {}", problem.headline(), problem.skill);
        print_detail(format!("scope: {}", problem.scope_label()));
        print_detail(format!("source: {}", problem.url));
        print_detail(format!("ref: {}", problem.reference));
        print_detail(format!("path: {}", problem.path));
        print_detail(format!("reason: {}", problem.reason()));
        print_detail(format!("suggestion: {}", problem.suggestion()));
    }
}

/// Read-only remote source probe used by `doctor --remote`.
///
/// Clones the repository into a temporary directory, checks out the configured ref,
/// and reports whether the configured path exists. The temporary clone is always
/// removed; no sksync state is mutated.
#[derive(Debug, Default)]
struct GitRemoteSourceChecker;

impl RemoteSourceChecker for GitRemoteSourceChecker {
    fn check_git_source(&self, source: &GitInstallSource) -> RemoteSourceStatus {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let clone_dir = std::env::temp_dir().join(format!(
            "sksync-doctor-remote-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&clone_dir);

        let status = match GitClient.clone_checkout(source, &clone_dir) {
            Ok(()) => {
                if clone_dir.join(&source.path).exists() {
                    RemoteSourceStatus::Available
                } else {
                    RemoteSourceStatus::PathMissing
                }
            }
            Err(error) => RemoteSourceStatus::RepoUnreachable(error.message),
        };
        let _ = fs::remove_dir_all(&clone_dir);
        status
    }
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
            store.add_dependency(
                &candidate.name,
                &source,
                &args.agents,
                AddDependencyOptions { include: None },
            )?;
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

fn run_bundle(args: BundleArgs) -> Result<()> {
    match args.command {
        BundleCommand::Inspect(args) => run_bundle_inspect(args),
        BundleCommand::Add(args) => run_bundle_add(args),
        BundleCommand::Remove(args) => run_bundle_remove(args),
        BundleCommand::Sync(args) => run_bundle_sync(args),
        BundleCommand::Export(args) => run_bundle_export(args),
    }
}

fn run_bundle_export(args: BundleExportArgs) -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let config_path = config_path_for(args.global, &current_dir)?;
    let lockfile_path = lockfile_path_for(args.global, &current_dir)?;
    let root_dir = if args.global {
        config_root_for_global()?
    } else {
        current_dir.clone()
    };
    let store = FileDependencyConfigStore::new(&config_path, default_skill_dir_for(args.global)?);
    let dependencies = store.load_bundle_export_dependencies()?;
    let resolved_config = load_config_from_path(&config_path, scope_for(args.global))?;
    let resolved_skills = resolved_config
        .skills
        .iter()
        .filter(|skill| skill.install_source.is_some())
        .map(|skill| BundleExportResolvedSkill {
            name: skill.name.as_str().to_owned(),
            source_path: skill.source.as_path().to_path_buf(),
        })
        .collect::<Vec<_>>();
    let destination = if args.root {
        BundleExportDestination::ManifestFile(root_dir.join(BUNDLE_MANIFEST_FILE))
    } else {
        BundleExportDestination::Directory(resolve_export_output_path(
            args.output
                .as_deref()
                .expect("clap requires --output without --root"),
            &root_dir,
        ))
    };
    let plan = build_bundle_export_plan(BundleExportPlanInput {
        name: args.name,
        description: None,
        destination,
        mode: if args.snapshot {
            BundleExportMode::Snapshot
        } else {
            BundleExportMode::ManifestOnly
        },
        selected_skills: args.skills,
        dependencies,
        resolved_skills,
    })?;
    if let BundleExportDestination::Directory(output) = &plan.destination {
        validate_bundle_export_output_safety(
            output,
            &root_dir,
            &config_path,
            &lockfile_path,
            resolved_config.skill_dir.as_path(),
        )?;
    }
    validate_bundle_export_plan(&plan)?;
    print_bundle_export_plan(&plan);
    if args.dry_run {
        return Ok(());
    }
    apply_bundle_export_plan(&plan, BundleExportApplyOptions { force: args.force })?;
    print_success(format!(
        "Exported bundle: {} -> {}",
        plan.manifest.name,
        plan.destination.path().display()
    ));
    Ok(())
}

fn resolve_export_output_path(output: &Path, root_dir: &Path) -> PathBuf {
    if output.is_absolute() {
        output.to_path_buf()
    } else {
        root_dir.join(output)
    }
}

fn validate_bundle_export_output_safety(
    output: &Path,
    root_dir: &Path,
    config_path: &Path,
    lockfile_path: &Path,
    skill_dir: &Path,
) -> Result<()> {
    let output = normalize_cli_path_for_compare(output);
    let root_dir = normalize_cli_path_for_compare(root_dir);
    let config_path = normalize_cli_path_for_compare(config_path);
    let lockfile_path = normalize_cli_path_for_compare(lockfile_path);
    let skill_dir = normalize_cli_path_for_compare(&expand_tilde_path(skill_dir));

    if output == root_dir {
        bail!("bundle export output must not be the active config root");
    }
    for protected in [&config_path, &lockfile_path, &skill_dir] {
        if output == *protected || protected.starts_with(&output) || output.starts_with(protected) {
            bail!(
                "bundle export output must not overlap protected sksync state: {}",
                protected.display()
            );
        }
    }
    Ok(())
}

fn expand_tilde_path(path: &Path) -> PathBuf {
    let value = path.to_string_lossy();
    if value == "~" {
        return dirs::home_dir().unwrap_or_else(|| path.to_path_buf());
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    path.to_path_buf()
}

fn normalize_cli_path_for_compare(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn print_bundle_export_plan(plan: &BundleExportPlan) {
    print_section("Bundle export plan");
    println!("Name: {}", plan.manifest.name);
    println!("Mode: {}", plan.mode.as_str());
    println!("Output: {}", plan.destination.path().display());
    print_section_with_count("Entries", plan.items.len());
    for item in &plan.items {
        match (&item.source_path, &item.snapshot_destination) {
            (Some(source), Some(destination)) => println!(
                "- {}: {} -> {}",
                item.skill_name,
                source.display(),
                destination.display()
            ),
            _ => println!("- {}: {}", item.skill_name, item.manifest_source),
        }
    }
}

fn run_bundle_inspect(args: BundleInspectArgs) -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    print_progress("Loading bundle manifest...");
    let candidates = discover_bundle_manifest_candidates(&args.source, &current_dir)?;
    let bundle = select_bundle_manifest_candidates(&args.source, args.name.as_deref(), candidates)?
        .into_loaded_bundle();

    print_section("Bundle");
    println!("Name: {}", bundle.manifest.name);
    println!("Description: {}", bundle.manifest.description);
    println!("Source: {}", bundle.provenance.source);
    print_section_with_count("Entries", bundle.entries.len());
    for entry in &bundle.entries {
        println!(
            "- {}: {} -> {}",
            entry.skill_name, entry.original_source, entry.normalized_source
        );
    }
    Ok(())
}

fn run_bundle_sync(args: BundleSyncArgs) -> Result<()> {
    parse_agent_kinds(&args.agents)?;
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let config_path = config_path_for(args.global, &current_dir)?;
    let lockfile_path = lockfile_path_for(args.global, &current_dir)?;
    let root_dir = if args.global {
        config_root_for_global()?
    } else {
        current_dir.clone()
    };
    let bundle_name = BundleName::new(args.name.clone()).context("invalid bundle name")?;
    let store = FileDependencyConfigStore::new(&config_path, default_skill_dir_for(args.global)?);
    let source = match store.resolve_bundle_sync_source(&bundle_name, args.source.as_deref())? {
        BundleSyncSourceResolution::Resolved(source) => source,
        BundleSyncSourceResolution::Ambiguous(sources) => {
            bail!(
                "bundle sync is ambiguous; pass --source <exact-source> (matches: {})",
                sources.join(", ")
            )
        }
        BundleSyncSourceResolution::NotFound => bail!("bundle provenance not found"),
    };
    print_progress("Loading bundle manifest...");
    let bundle = load_bundle_from_source(&source, &root_dir)?;
    if bundle.manifest.name != bundle_name {
        bail!(
            "bundle manifest name changed from {} to {}; aborting sync",
            bundle_name,
            bundle.manifest.name
        );
    }
    let provenance = crate::domain::bundle::BundleProvenance {
        name: bundle_name,
        source,
    };
    print_progress("Planning changes...");
    let plan = store.plan_bundle_sync(&provenance, &bundle.entries, &args.agents)?;
    print_bundle_sync_plan(&plan);

    if args.dry_run {
        return Ok(());
    }
    if plan.has_blockers() {
        bail!("bundle sync has blocking item(s); rerun with --dry-run for details")
    }
    let add_plan = bundle_sync_add_plan(&plan, &provenance);
    let remove_plan = bundle_sync_remove_plan(&plan);
    if add_plan.items.is_empty() && remove_plan.items.is_empty() && !args.force {
        print_success(format!("Bundle already synchronized: {}", plan.bundle));
        return Ok(());
    }

    let config_backup = ConfigFileBackup::capture(&config_path)?;
    let lockfile_backup = ConfigFileBackup::capture(&lockfile_path)?;
    let created_skill_names = add_plan
        .items
        .iter()
        .filter(|item| item.status == BundleAddStatus::Create)
        .map(|item| item.skill_name.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let mut created_skill_dirs = Vec::new();
    let mut created_link_targets = Vec::new();
    let sync_result = (|| -> Result<()> {
        if !add_plan.items.is_empty() {
            store.apply_bundle_add(&add_plan)?;
            let mut config = load_config_from_path(&config_path, scope_for(args.global))?;
            created_skill_dirs = config
                .skills
                .iter()
                .filter(|skill| created_skill_names.contains(skill.name.as_str()))
                .filter_map(|skill| {
                    let path = skill.source.as_path();
                    if path.exists() {
                        None
                    } else {
                        Some(path.to_path_buf())
                    }
                })
                .collect();
            print_progress("Installing skills...");
            let update_report = update_dependencies(&config, &FileSystemSkillInstaller)?;
            apply_update_report_sources(&mut config, &update_report);
            print_update_report(update_report);
        }

        if !remove_plan.items.is_empty() {
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
            print_progress("Removing bundle-managed skills...");
            store.detach_bundle_provenance(&remove_plan)?;
            let remove_args = RemoveArgs {
                skills: Vec::new(),
                global: args.global,
                config_only: false,
                agents: Vec::new(),
                keep_files: false,
            };
            for item in &remove_plan.items {
                if item.status == BundleRemoveStatus::Remove {
                    let skill = config
                        .skills
                        .iter()
                        .find(|skill| skill.name.as_str() == item.skill_name);
                    remove_entire_skill(
                        &remove_args,
                        &item.skill_name,
                        skill.map(|skill| skill.source.as_path().to_path_buf()),
                        &mut lockfile,
                        &runtime,
                    )?;
                }
            }
        }

        if !add_plan.items.is_empty() || args.force {
            let config = load_config_from_path(&config_path, scope_for(args.global))?;
            let fs_store = FileSystemLinkStore;
            let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
            let target_resolver = TargetPathResolver::new(&root_dir, home_dir);
            let link_plan = build_link_plan(&config, &fs_store, &fs_store, &target_resolver)?;
            created_link_targets = link_plan
                .items
                .iter()
                .filter(|item| matches!(item.action, PlanAction::CreateSymlink))
                .map(|item| item.target.as_path().to_path_buf())
                .filter(|target| !target.exists())
                .collect();
            let lockfile = build_lockfile_from_plan(&config, &link_plan, &root_dir)?;
            print_progress("Applying links...");
            apply_link_plan(
                &link_plan,
                &lockfile,
                &fs_store,
                &FileLockfileStore::new(&lockfile_path),
                ApplyOptions {
                    force: args.force,
                    skip_blocked_targets: true,
                },
            )?;
            print_plan(&link_plan);
        }

        print_success(format!("Synchronized bundle: {}", plan.bundle));
        Ok(())
    })();

    if let Err(error) = sync_result {
        cleanup_bundle_add_artifacts(&created_link_targets, &created_skill_dirs);
        let config_restore = config_backup.restore();
        let lockfile_restore = lockfile_backup.restore();
        if let Err(restore_error) = config_restore.and(lockfile_restore) {
            return Err(error.context(format!(
                "sksync bundle sync failed and rollback failed: {restore_error}"
            )));
        }
        return Err(
            error.context("sksync bundle sync failed; restored previous config and lockfile")
        );
    }

    Ok(())
}

fn bundle_sync_add_plan(
    plan: &BundleSyncPlan,
    provenance: &crate::domain::bundle::BundleProvenance,
) -> BundleAddPlan {
    BundleAddPlan {
        items: plan
            .items
            .iter()
            .filter_map(|item| {
                let status = match item.status {
                    BundleSyncStatus::Add => BundleAddStatus::Create,
                    BundleSyncStatus::Adopt | BundleSyncStatus::IncludeChanged => {
                        BundleAddStatus::Merge
                    }
                    _ => return None,
                };
                Some(BundleAddPlanItem {
                    skill_name: item.skill_name.clone(),
                    source: item.manifest_source.clone().unwrap_or_default(),
                    include: item.include.clone(),
                    agents: item.agents.clone(),
                    provenance: provenance.clone(),
                    status,
                    message: item.message.clone(),
                })
            })
            .collect(),
    }
}

fn bundle_sync_remove_plan(plan: &BundleSyncPlan) -> BundleRemovePlan {
    BundleRemovePlan {
        bundle: plan.bundle.clone(),
        source: Some(plan.source.clone()),
        items: plan
            .items
            .iter()
            .filter_map(|item| {
                let status = match item.status {
                    BundleSyncStatus::Remove => BundleRemoveStatus::Remove,
                    BundleSyncStatus::DetachProvenance => BundleRemoveStatus::DetachProvenance,
                    _ => return None,
                };
                Some(BundleRemovePlanItem {
                    skill_name: item.skill_name.clone(),
                    status,
                    source: item.local_source.clone(),
                    message: item.message.clone(),
                })
            })
            .collect(),
        ambiguous_sources: Vec::new(),
    }
}

fn run_bundle_add(args: BundleAddArgs) -> Result<()> {
    parse_agent_kinds(&args.agents)?;
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let config_path = config_path_for(args.global, &current_dir)?;
    let lockfile_path = lockfile_path_for(args.global, &current_dir)?;
    let root_dir = if args.global {
        config_root_for_global()?
    } else {
        current_dir.clone()
    };
    print_progress("Loading bundle manifest...");
    let candidates = discover_bundle_manifest_candidates(&args.source, &root_dir)?;
    let bundle = select_bundle_manifest_candidates(&args.source, args.name.as_deref(), candidates)?
        .into_loaded_bundle();
    let store = FileDependencyConfigStore::new(&config_path, default_skill_dir_for(args.global)?);
    print_progress("Planning changes...");
    let plan = store.plan_bundle_add(&bundle.entries, &args.agents, &bundle.provenance)?;
    print_bundle_add_plan(&plan);

    if args.dry_run {
        if plan.has_conflicts() {
            bail!("bundle add has conflict(s)");
        }
        return Ok(());
    }
    if plan.has_conflicts() {
        bail!("bundle add has conflict(s); rerun with --dry-run for details")
    }

    let config_backup = ConfigFileBackup::capture(&config_path)?;
    let lockfile_backup = ConfigFileBackup::capture(&lockfile_path)?;
    let created_skill_names = plan
        .items
        .iter()
        .filter(|item| item.status == crate::application::bundle::BundleAddStatus::Create)
        .map(|item| item.skill_name.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let mut created_skill_dirs = Vec::new();
    let mut created_link_targets = Vec::new();
    let add_result = (|| -> Result<()> {
        store.apply_bundle_add(&plan)?;
        let mut config = load_config_from_path(&config_path, scope_for(args.global))?;
        created_skill_dirs = config
            .skills
            .iter()
            .filter(|skill| created_skill_names.contains(skill.name.as_str()))
            .filter_map(|skill| {
                let path = skill.source.as_path();
                if path.exists() {
                    None
                } else {
                    Some(path.to_path_buf())
                }
            })
            .collect();
        print_progress("Installing skills...");
        let update_report = update_dependencies(&config, &FileSystemSkillInstaller)?;
        apply_update_report_sources(&mut config, &update_report);
        let fs_store = FileSystemLinkStore;
        let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
        let target_resolver = TargetPathResolver::new(&root_dir, home_dir);
        let link_plan = build_link_plan(&config, &fs_store, &fs_store, &target_resolver)?;
        created_link_targets = link_plan
            .items
            .iter()
            .filter(|item| matches!(item.action, PlanAction::CreateSymlink))
            .map(|item| item.target.as_path().to_path_buf())
            .filter(|target| !target.exists())
            .collect();
        let lockfile = build_lockfile_from_plan(&config, &link_plan, &root_dir)?;
        print_progress("Applying links...");
        apply_link_plan(
            &link_plan,
            &lockfile,
            &fs_store,
            &FileLockfileStore::new(&lockfile_path),
            ApplyOptions {
                force: args.force,
                skip_blocked_targets: true,
            },
        )?;
        print_success(format!("Added bundle: {}", bundle.manifest.name));
        print_update_report(update_report);
        print_plan(&link_plan);
        Ok(())
    })();

    if let Err(error) = add_result {
        cleanup_bundle_add_artifacts(&created_link_targets, &created_skill_dirs);
        let config_restore = config_backup.restore();
        let lockfile_restore = lockfile_backup.restore();
        if let Err(restore_error) = config_restore.and(lockfile_restore) {
            return Err(error.context(format!(
                "sksync bundle add failed and rollback failed: {restore_error}"
            )));
        }
        return Err(
            error.context("sksync bundle add failed; restored previous config and lockfile")
        );
    }

    Ok(())
}

fn cleanup_bundle_add_artifacts(link_targets: &[PathBuf], skill_dirs: &[PathBuf]) {
    for target in link_targets.iter().rev() {
        if fs::symlink_metadata(target)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            let _ = fs::remove_file(target);
        }
    }
    for dir in skill_dirs.iter().rev() {
        if dir.is_dir() {
            let _ = fs::remove_dir_all(dir);
        }
    }
}

fn run_bundle_remove(args: BundleRemoveArgs) -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let config_path = config_path_for(args.global, &current_dir)?;
    let lockfile_path = lockfile_path_for(args.global, &current_dir)?;
    let bundle_name = BundleName::new(args.name.clone()).context("invalid bundle name")?;
    let store = FileDependencyConfigStore::new(&config_path, default_skill_dir_for(args.global)?);
    let plan = store.plan_bundle_remove(&bundle_name, args.source.as_deref())?;
    print_bundle_remove_plan(&plan);

    if args.dry_run {
        if plan.is_ambiguous() {
            bail!("bundle remove is ambiguous; pass --source <exact-source>");
        }
        return Ok(());
    }
    if plan.is_ambiguous() {
        bail!("bundle remove is ambiguous; pass --source <exact-source>")
    }
    if plan.is_not_found() {
        bail!("bundle provenance not found")
    }

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
        print_progress("Removing bundle-managed skills...");
        store.detach_bundle_provenance(&plan)?;
        let remove_args = RemoveArgs {
            skills: Vec::new(),
            global: args.global,
            config_only: false,
            agents: Vec::new(),
            keep_files: false,
        };
        for item in &plan.items {
            if item.status == BundleRemoveStatus::Remove {
                let skill = config
                    .skills
                    .iter()
                    .find(|skill| skill.name.as_str() == item.skill_name);
                remove_entire_skill(
                    &remove_args,
                    &item.skill_name,
                    skill.map(|skill| skill.source.as_path().to_path_buf()),
                    &mut lockfile,
                    &runtime,
                )?;
            }
        }
        print_success(format!("Removed bundle provenance: {bundle_name}"));
        Ok(())
    })();

    if let Err(error) = remove_result {
        let config_restore = config_backup.restore();
        let lockfile_restore = lockfile_backup.restore();
        if let Err(restore_error) = config_restore.and(lockfile_restore) {
            return Err(error.context(format!(
                "sksync bundle remove failed and rollback failed: {restore_error}"
            )));
        }
        return Err(
            error.context("sksync bundle remove failed; restored previous config and lockfile")
        );
    }

    Ok(())
}

fn print_bundle_add_plan(plan: &BundleAddPlan) {
    print_section_with_count("Bundle add plan", plan.items.len());
    for item in &plan.items {
        println!(
            "{} {} <- {}",
            item.status.as_str(),
            item.skill_name,
            item.source
        );
        if let Some(message) = &item.message {
            println!("  ! {message}");
        }
    }
}

fn print_bundle_remove_plan(plan: &BundleRemovePlan) {
    print_section_with_count("Bundle remove plan", plan.items.len());
    for item in &plan.items {
        let source = item
            .source
            .as_deref()
            .or(plan.source.as_deref())
            .unwrap_or("*");
        println!("{} {} ({source})", item.status.as_str(), item.skill_name);
        if let Some(message) = &item.message {
            println!("  ! {message}");
        }
    }
}

fn print_bundle_sync_plan(plan: &BundleSyncPlan) {
    print_section_with_count("Bundle sync plan", plan.items.len());
    println!("Bundle: {}", plan.bundle);
    println!("Source: {}", plan.source);
    println!("keep: {}", plan.keep_count);
    for item in &plan.items {
        match (&item.local_source, &item.manifest_source) {
            (Some(local), Some(manifest)) => println!(
                "{} {} (local: {}, manifest: {})",
                item.status.as_str(),
                item.skill_name,
                local,
                manifest
            ),
            (Some(local), None) => {
                println!("{} {} ({})", item.status.as_str(), item.skill_name, local)
            }
            (None, Some(manifest)) => println!(
                "{} {} <- {}",
                item.status.as_str(),
                item.skill_name,
                manifest
            ),
            (None, None) => println!("{} {}", item.status.as_str(), item.skill_name),
        }
        if !item.agents.is_empty() {
            println!("  agents: {}", item.agents.join(", "));
        }
        if let Some(message) = &item.message {
            println!("  ! {message}");
        }
    }
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
    print_progress("Resolving skill source...");
    let include = package_filter_from_add_args(&args)?;
    let existing_names =
        FileDependencyConfigStore::new(&config_path, default_skill_dir_for(args.global)?)
            .existing_dependency_names()?;
    let selections = match resolve_add_selections(
        &args.source,
        args.name.as_deref(),
        &config_path,
        include,
        &existing_names,
    )? {
        AddSelectionOutcome::AllInstalled => {
            print_success(
                "All discovered skills are already installed; nothing to add. Use `sksync attach`, `sksync update`, or `sksync remove` to manage existing dependencies.",
            );
            return Ok(());
        }
        AddSelectionOutcome::Selections(selections) => selections,
    };
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
        print_progress("Installing skills...");
        let report = run_add_workflow(
            selections,
            &args.agents,
            args.force,
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
        print_progress("Installing skills...");
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
        print_progress("Applying links...");
        apply_link_plan(
            &plan,
            &lockfile,
            &fs_store,
            &FileLockfileStore::new(lockfile_path_for(args.global, &current_dir)?),
            ApplyOptions {
                force: args.force,
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

fn should_remove_target_for_skill(item: &LinkPlanItem, skill: &str) -> bool {
    item.owners
        .iter()
        .any(|owner| owner.skill.as_str() == skill)
        && item
            .owners
            .iter()
            .all(|owner| owner.skill.as_str() == skill)
}

fn should_remove_target_for_agents(
    item: &LinkPlanItem,
    skill: &str,
    agents: &[AgentKind],
) -> bool {
    let removes_owner = |owner: &crate::domain::link_plan::LinkOwner| {
        owner.skill.as_str() == skill && agent_kinds_contain(agents, &owner.agent)
    };
    item.owners.iter().any(removes_owner) && item.owners.iter().all(removes_owner)
}

fn remove_managed_symlinks(plan: &LinkPlan, skill: &str) -> Result<()> {
    for item in &plan.items {
        if should_remove_target_for_skill(item, skill) {
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
        if should_remove_target_for_agents(item, skill, agents) {
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
    print_progress("Planning links...");
    let (_config, plan, _current_dir) = load_plan(args.global)?;
    print_plan(&plan);
    Ok(())
}

fn run_apply(args: ApplyArgs) -> Result<()> {
    print_progress("Planning links...");
    let (config, plan, current_dir) = load_plan(args.global)?;
    let lockfile = build_lockfile_from_plan(&config, &plan, &current_dir)?;
    let fs_store = FileSystemLinkStore;
    let lockfile_store = FileLockfileStore::new(lockfile_path_for(args.global, &current_dir)?);

    print_progress("Applying links...");
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
    print_progress("Installing skills...");
    let report = update_dependencies(&config, &FileSystemSkillInstaller)?;
    apply_update_report_sources(&mut config, &report);
    print_update_report(report);
    print_progress("Planning links...");
    let (config, plan, root_dir) = build_plan_from_config(config, args.global, &current_dir)?;
    let lockfile = build_lockfile_from_plan(&config, &plan, &root_dir)?;
    let fs_store = FileSystemLinkStore;
    let lockfile_store = FileLockfileStore::new(lockfile_path);
    print_progress("Applying links...");
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

fn run_update(args: UpdateArgs) -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let mut config = load_config_for_scope(args.global, &current_dir)?;
    print_progress("Installing skills...");
    let report = update_dependencies(&config, &FileSystemSkillInstaller)?;
    apply_update_report_sources(&mut config, &report);
    print_update_report(report);
    print_progress("Planning links...");
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
            skill.include = locked.include.clone();
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
struct BundleChoice(BundleManifestCandidate);

impl std::fmt::Display for BundleChoice {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}  {}",
            self.0.manifest.name,
            self.0.relative_path.display()
        )
    }
}

fn score_bundle_choice(
    input: &str,
    choice: &BundleChoice,
    _display: &str,
    _index: usize,
) -> Option<i64> {
    let filter = input.trim().to_lowercase();
    if filter.is_empty() {
        return Some(0);
    }

    let name = choice.0.manifest.name.as_str().to_lowercase();
    let path = choice.0.relative_path.to_string_lossy().to_lowercase();
    let description = choice.0.manifest.description.to_lowercase();

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

fn bundle_candidate_matches_name(candidate: &BundleManifestCandidate, name: &str) -> bool {
    candidate.manifest.name.as_str() == name
        || candidate
            .relative_path
            .file_name()
            .and_then(|file_name| file_name.to_str())
            == Some(name)
        || Path::new(&candidate.resolved_source)
            .file_name()
            .and_then(|file_name| file_name.to_str())
            == Some(name)
}

fn bundle_candidate_rows(candidates: &[BundleManifestCandidate]) -> String {
    candidates
        .iter()
        .map(|candidate| {
            format!(
                "- {} ({})",
                candidate.relative_path.display(),
                candidate.manifest.name
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn select_bundle_manifest_candidates(
    source: &str,
    requested_name: Option<&str>,
    candidates: Vec<BundleManifestCandidate>,
) -> Result<BundleManifestCandidate> {
    if candidates.is_empty() {
        bail!("no sksync.bundle.json files found under source '{source}'");
    }

    if let Some(name) = requested_name {
        let matches = candidates
            .into_iter()
            .filter(|candidate| bundle_candidate_matches_name(candidate, name))
            .collect::<Vec<_>>();
        return match matches.as_slice() {
            [candidate] => Ok(candidate.clone()),
            [] => bail!("no discovered bundle named '{name}' under source '{source}'"),
            _ => bail!(
                "multiple discovered bundles matched '{name}' under source '{source}':\n{}",
                bundle_candidate_rows(&matches)
            ),
        };
    }

    if candidates.len() == 1 {
        return Ok(candidates.into_iter().next().expect("one candidate"));
    }

    if !std::io::stdin().is_terminal() {
        bail!(
            "multiple bundles found under source '{source}'; pass --name <bundle> or use a more specific source:\n{}",
            bundle_candidate_rows(&candidates)
        );
    }

    let choices = candidates.into_iter().map(BundleChoice).collect::<Vec<_>>();
    Ok(inquire::Select::new("Select bundle manifest", choices)
        .with_scorer(&score_bundle_choice)
        .with_help_message("type: filter name/path/description · enter: confirm")
        .prompt()?
        .0)
}

#[derive(Debug, Clone)]
struct SkillChoice {
    candidate: SkillCandidate,
    already_installed: bool,
}

impl std::fmt::Display for SkillChoice {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}  {}",
            self.candidate.name,
            self.candidate.relative_path.display()
        )?;
        if self.already_installed {
            formatter.write_str("  (already installed)")?;
        }
        Ok(())
    }
}

/// Build the interactive multi-select model, marking candidates whose skill name
/// already exists in config as `already installed`. The selection prompt keeps
/// these rows visible but rejects selecting them.
fn build_skill_choices(
    candidates: Vec<SkillCandidate>,
    existing_names: &BTreeSet<String>,
) -> Vec<SkillChoice> {
    candidates
        .into_iter()
        .map(|candidate| SkillChoice {
            already_installed: existing_names.contains(&candidate.name),
            candidate,
        })
        .collect()
}

fn format_selected_skill_choices(
    selected: &[inquire::list_option::ListOption<&SkillChoice>],
) -> String {
    let names = selected
        .iter()
        .map(|option| option.value.candidate.name.as_str())
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

    let name = choice.candidate.name.to_lowercase();
    let path = choice
        .candidate
        .relative_path
        .to_string_lossy()
        .to_lowercase();
    let description = choice.candidate.description.to_lowercase();

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

fn package_filter_from_add_args(args: &AddArgs) -> Result<Option<PackageFilter>> {
    if args.manifest_only {
        return Ok(Some(PackageFilter::manifest_only()));
    }
    if args.include.is_empty() {
        return Ok(None);
    }
    Ok(Some(PackageFilter::new(args.include.clone())?))
}

/// Outcome of resolving which skills `add` should create.
///
/// `AllInstalled` means every discovered candidate is already a configured
/// dependency, so `add` exits without mutating config, files, or the lockfile.
enum AddSelectionOutcome {
    Selections(Vec<AddSelection>),
    AllInstalled,
}

#[derive(Debug)]
enum CandidateSelection {
    Selected(Vec<SkillCandidate>),
    AllInstalled,
}

fn resolve_add_selections(
    source: &str,
    requested_name: Option<&str>,
    config_path: &Path,
    include: Option<PackageFilter>,
    existing_names: &BTreeSet<String>,
) -> Result<AddSelectionOutcome> {
    let config_root = config_path.parent().unwrap_or_else(|| Path::new("."));
    let fallback_name = infer_skill_name(source);
    let parse_skill_name = requested_name.unwrap_or(&fallback_name);
    let install_source = parse_install_source_value(parse_skill_name, source, Some(config_root))
        .with_context(|| format!("failed to parse source '{source}'"))?;

    let discovered = discover_source_skills(&install_source, source)?;
    let selection_name = requested_name.or(discovered.default_selection_name.as_deref());
    let selections = match select_skill_candidates(
        source,
        selection_name,
        discovered.candidates,
        existing_names,
    )? {
        CandidateSelection::AllInstalled => return Ok(AddSelectionOutcome::AllInstalled),
        CandidateSelection::Selected(selections) => selections,
    };

    Ok(AddSelectionOutcome::Selections(
        selections
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
                include: include.clone(),
            })
            .collect(),
    ))
}

fn select_skill_candidates(
    source: &str,
    requested_name: Option<&str>,
    candidates: Vec<SkillCandidate>,
    existing_names: &BTreeSet<String>,
) -> Result<CandidateSelection> {
    if candidates.is_empty() {
        bail!("no SKILL.md files found under source '{source}'");
    }

    if let Some(name) = requested_name {
        if existing_names.contains(name) {
            bail!(already_installed_message(name));
        }

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
            [candidate] => {
                if existing_names.contains(&candidate.name) {
                    bail!(already_installed_message(&candidate.name));
                }
                Ok(CandidateSelection::Selected(vec![candidate.clone()]))
            }
            [] => bail!("no discovered skill named '{name}' under source '{source}'"),
            _ => bail!("multiple discovered skills matched '{name}' under source '{source}'"),
        };
    }

    // A single discovered skill is the targeted dependency. If it already exists,
    // adding it again is a duplicate add and fails clearly with guidance.
    if candidates.len() == 1 {
        let candidate = candidates.into_iter().next().expect("one candidate");
        if existing_names.contains(&candidate.name) {
            bail!(already_installed_message(&candidate.name));
        }
        return Ok(CandidateSelection::Selected(vec![candidate]));
    }

    // Multiple discovered skills: candidates already in config are existing
    // dependencies. If every candidate is already installed there is nothing new
    // to add, so the flow exits without changes.
    let installable = candidates
        .iter()
        .filter(|candidate| !existing_names.contains(&candidate.name))
        .count();
    if installable == 0 {
        return Ok(CandidateSelection::AllInstalled);
    }

    if !std::io::stdin().is_terminal() {
        bail!(
            "multiple skills found under source '{source}'; pass --name <skill> or use a more specific source"
        );
    }

    let choices = build_skill_choices(candidates, existing_names);
    let selected = inquire::MultiSelect::new("Select skills to add", choices)
        .with_formatter(&format_selected_skill_choices)
        .with_scorer(&score_skill_choice)
        .with_validator(
            |selected: &[inquire::list_option::ListOption<&SkillChoice>]| {
                let installed = selected
                    .iter()
                    .filter(|option| option.value.already_installed)
                    .map(|option| option.value.candidate.name.clone())
                    .collect::<Vec<_>>();
                if installed.is_empty() {
                    Ok(inquire::validator::Validation::Valid)
                } else {
                    Ok(inquire::validator::Validation::Invalid(
                        format!("already installed, cannot select: {}", installed.join(", "))
                            .into(),
                    ))
                }
            },
        )
        .with_help_message("space: select · type: filter name/path/description · enter: confirm")
        .prompt()?;
    if selected.is_empty() {
        bail!("no skills selected");
    }
    Ok(CandidateSelection::Selected(
        selected
            .into_iter()
            .map(|choice| choice.candidate)
            .collect(),
    ))
}

fn already_installed_message(skill_name: &str) -> String {
    format!(
        "skill '{skill_name}' is already installed; use `sksync attach {skill_name} --agent <agent>` to add agents, `sksync update` to refresh it, or `sksync remove {skill_name}` to replace it"
    )
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
    println!(
        "{badge:<8} {} → {}",
        item.skill_label(),
        item.agent_label()
    );
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
    for group in group_check_problems(problems) {
        print_problem_group(&group);
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

fn print_progress(message: impl AsRef<str>) {
    eprintln!(
        "{}",
        format_progress_message(message.as_ref(), std::io::stderr().is_terminal())
    );
}

fn format_progress_message(message: &str, color: bool) -> String {
    let plain = format!("→ {message}");
    if color {
        format!("\u{1b}[36m{plain}\u{1b}[0m")
    } else {
        plain
    }
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
                include: skill.include.clone(),
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
    let report = check_lockfile_with_config_and_plan(
        &config,
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
        agent_mapping_group, agent_target_mappings_from_config, apply_locked_install_sources,
        build_doctor_sections, build_local_problem_groups, compact_revision, compact_source,
        copy_dir_all, doctor_remote_problems, format_progress_message,
        format_selected_skill_choices, global_config_root_from_home, is_managed_skill_dir,
        list_state_label, reject_legacy_registry_source, remove_installed_skill_dir,
        scan_import_candidates, score_skill_choice, select_bundle_manifest_candidates,
        select_skill_candidates, truncate_middle, Cli, Command, ConfigFileBackup, DoctorSection,
        GitRemoteSourceChecker, PlanProblems,
    };
    use crate::application::bundle::BundleManifestCandidate;
    use crate::application::check::CheckProblem;
    use crate::application::config::{ResolvedConfig, ResolvedSkill};
    use crate::application::discovery::{
        discover_skill_candidates, source_with_selected_subpath, SourceRewriteMode,
    };
    use crate::application::remote::RemoteSourceProblem;
    use crate::application::remote::{
        collect_remote_source_problems, RemoteSourceChecker, RemoteSourceProblemKind,
        RemoteSourceStatus,
    };
    use crate::domain::agent::AgentKind;
    use crate::domain::bundle::{BundleManifest, BundleName, BundleProvenance};
    use crate::domain::link_plan::{LinkOwner, LinkPlanItem, PlanAction};
    use crate::domain::lockfile::{Digest, LockedSkill, Lockfile};
    use crate::domain::package_filter::PackageFilter;
    use crate::domain::scope::Scope;
    use crate::domain::skill::{SkillName, SourcePath};
    use crate::domain::source::GitInstallSource;
    use crate::domain::source::InstallSource;
    use crate::infrastructure::json::AgentMappingConfig;
    use clap::{CommandFactory, Parser};
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::{Path, PathBuf};

    fn remote_problem(skill: &str) -> RemoteSourceProblem {
        RemoteSourceProblem {
            skill: skill.to_owned(),
            global: false,
            url: "https://example.com/repo.git".to_owned(),
            reference: "HEAD".to_owned(),
            path: "skills/x".to_owned(),
            kind: RemoteSourceProblemKind::PathMissing,
        }
    }

    fn target_conflict(skill: &str, agent: &str, path: &str) -> CheckProblem {
        CheckProblem::TargetConflict {
            skill: skill.to_owned(),
            agent: agent.to_owned(),
            path: path.to_owned(),
            reason: "regular file exists".to_owned(),
        }
    }

    #[test]
    fn doctor_sections_print_remote_before_local_groups() {
        let remote = vec![remote_problem("caveman")];
        let local = build_local_problem_groups(
            &[],
            &PlanProblems::default(),
            &[target_conflict("review", "pi", ".pi/agent/skills/review")],
        );

        let sections = build_doctor_sections(&remote, local, None, None);

        let titles: Vec<&str> = sections.iter().map(DoctorSection::title).collect();
        assert_eq!(titles.first().copied(), Some("Remote source problems"));
        let remote_index = titles
            .iter()
            .position(|title| *title == "Remote source problems")
            .unwrap();
        let conflict_index = titles
            .iter()
            .position(|title| *title == "Target conflicts")
            .unwrap();
        assert!(
            remote_index < conflict_index,
            "remote problems must print before local groups: {titles:?}"
        );
    }

    #[test]
    fn doctor_sections_keep_agent_warnings_in_separate_section() {
        let local = build_local_problem_groups(
            &[],
            &PlanProblems::default(),
            &[target_conflict("review", "pi", ".pi/agent/skills/review")],
        );
        let warnings = agent_mapping_group(
            "Agent mapping warnings",
            &["project · codex · /tmp/x: missing".to_owned()],
        );

        let sections = build_doctor_sections(&[], local, None, warnings);

        let titles: Vec<&str> = sections.iter().map(DoctorSection::title).collect();
        assert!(titles.contains(&"Target conflicts"));
        assert!(titles.contains(&"Agent mapping warnings"));
        // The agent-mapping section is distinct from any local/link group.
        let warning_section = sections
            .iter()
            .find(|section| section.title() == "Agent mapping warnings")
            .unwrap();
        if let DoctorSection::Group(group) = warning_section {
            assert!(group.hint.contains("sksync agents doctor"));
        } else {
            panic!("agent warnings should be a grouped section");
        }
    }

    #[test]
    fn local_problem_groups_separate_check_from_plan_findings() {
        let plan = PlanProblems {
            source_missing: Vec::new(),
            conflicts: vec![("legacy".to_owned(), "pi".to_owned(), "blocked".to_owned())],
            drift: Vec::new(),
        };
        let check = vec![target_conflict("review", "pi", ".pi/agent/skills/review")];

        let groups = build_local_problem_groups(&[], &plan, &check);

        let titles: Vec<&str> = groups.iter().map(|group| group.title.as_str()).collect();
        // Lockfile/link conflicts and plan-level conflicts stay in distinct sections.
        assert!(titles.contains(&"Target conflicts"));
        assert!(titles.contains(&"Plan conflicts"));
        // Counting across all groups matches the underlying problem count.
        let total: usize = groups.iter().map(|group| group.count).sum();
        assert_eq!(total, check.len() + plan.len());
    }

    #[test]
    fn agent_mapping_group_is_empty_when_no_lines() {
        assert!(agent_mapping_group("Agent mapping problems", &[]).is_none());
    }

    #[test]
    fn locked_install_sources_apply_include_filters() {
        let mut config = ResolvedConfig {
            skill_dir: SourcePath::new(".sksync/skills").unwrap(),
            agents: BTreeMap::new(),
            skills: vec![ResolvedSkill {
                name: SkillName::new("review").unwrap(),
                source: SourcePath::new(".sksync/skills/review").unwrap(),
                install_source: Some(InstallSource::Local(PathBuf::from("old"))),
                include: None,
                agents: Vec::new(),
            }],
            default_agents: Vec::new(),
        };
        let lockfile = Lockfile {
            generated_by: "test".to_owned(),
            generated_at: "test".to_owned(),
            root: PathBuf::from("."),
            skills: BTreeMap::from([(
                SkillName::new("review").unwrap(),
                LockedSkill {
                    source: SourcePath::new(".sksync/skills/review").unwrap(),
                    install_source: Some(InstallSource::Local(PathBuf::from("locked"))),
                    include: Some(PackageFilter::manifest_only()),
                    hash: Digest::new("sha256-test").unwrap(),
                    files: Vec::new(),
                    targets: Vec::new(),
                },
            )]),
        };

        apply_locked_install_sources(&mut config, &lockfile);

        assert_eq!(
            config.skills[0].install_source,
            Some(InstallSource::Local(PathBuf::from("locked")))
        );
        assert_eq!(
            config.skills[0].include,
            Some(PackageFilter::manifest_only())
        );
    }

    #[test]
    fn progress_message_is_colored_only_for_terminal_stderr() {
        assert_eq!(
            format_progress_message("Installing skills...", false),
            "→ Installing skills..."
        );
        assert_eq!(
            format_progress_message("Installing skills...", true),
            "\u{1b}[36m→ Installing skills...\u{1b}[0m"
        );
    }

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
                "init", "add", "attach", "agents", "doctor", "import", "bundle", "remove",
                "outdated", "plan", "apply", "install", "update", "check", "list", "wizard",
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
        Cli::try_parse_from(["sksync", "doctor", "--remote"]).expect("doctor --remote parses");
        Cli::try_parse_from(["sksync", "doctor", "--remote", "--global"])
            .expect("doctor --remote --global parses");
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

    fn bundle_candidate_for_test(name: &str, relative_path: &str) -> BundleManifestCandidate {
        let bundle_name = BundleName::new(name).unwrap();
        BundleManifestCandidate {
            manifest: BundleManifest {
                name: bundle_name.clone(),
                description: format!("{name} description"),
                entries: Vec::new(),
            },
            provenance: BundleProvenance {
                name: bundle_name,
                source: format!("./{relative_path}"),
            },
            entries: Vec::new(),
            relative_path: PathBuf::from(relative_path),
            resolved_source: format!("./{relative_path}"),
        }
    }

    #[test]
    fn bundle_add_accepts_name_selector() {
        Cli::try_parse_from([
            "sksync",
            "bundle",
            "add",
            "owner/repo",
            "--name",
            "team-baseline",
            "--agent",
            "pi",
        ])
        .expect("bundle add --name parses");
    }

    #[test]
    fn bundle_inspect_accepts_name_selector() {
        Cli::try_parse_from([
            "sksync",
            "bundle",
            "inspect",
            "owner/repo",
            "--name",
            "team-baseline",
        ])
        .expect("bundle inspect --name parses");
    }

    #[test]
    fn bundle_name_selector_matches_manifest_name_or_parent_dir() {
        let selected = select_bundle_manifest_candidates(
            "owner/repo",
            Some("base"),
            vec![
                bundle_candidate_for_test("review-workflow", "bundles/review"),
                bundle_candidate_for_test("team-baseline", "bundles/base"),
            ],
        )
        .expect("select candidate");

        assert_eq!(selected.relative_path, Path::new("bundles/base"));
    }

    #[test]
    fn bundle_name_selector_matches_manifest_name() {
        let selected = select_bundle_manifest_candidates(
            "owner/repo",
            Some("team-baseline"),
            vec![bundle_candidate_for_test("team-baseline", "bundles/base")],
        )
        .expect("select candidate");

        assert_eq!(selected.manifest.name.as_str(), "team-baseline");
    }

    #[test]
    fn bundle_name_selector_matches_resolved_parent_dir_for_direct_manifest() {
        let mut candidate = bundle_candidate_for_test("team-baseline", ".");
        candidate.resolved_source = "./repo/bundles/base".to_owned();
        candidate.provenance.source = candidate.resolved_source.clone();

        let selected =
            select_bundle_manifest_candidates("./repo/bundles/base", Some("base"), vec![candidate])
                .expect("select candidate");

        assert_eq!(selected.resolved_source, "./repo/bundles/base");
    }

    #[test]
    fn bundle_name_selector_rejects_multiple_matches() {
        let error = select_bundle_manifest_candidates(
            "owner/repo",
            Some("base"),
            vec![
                bundle_candidate_for_test("team-baseline", "bundles/base"),
                bundle_candidate_for_test("other", "examples/base"),
            ],
        )
        .expect_err("multiple matches should fail");

        assert!(error
            .to_string()
            .contains("multiple discovered bundles matched"));
        assert!(error.to_string().contains("bundles/base"));
        assert!(error.to_string().contains("examples/base"));
    }

    #[test]
    fn bundle_export_command_is_registered() {
        Cli::try_parse_from([
            "sksync",
            "bundle",
            "export",
            "team-baseline",
            "--output",
            "./bundles/team-baseline",
            "--skill",
            "review",
            "--snapshot",
            "--dry-run",
            "--force",
        ])
        .expect("bundle export should parse");
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

    fn skill_choice(name: &str, description: &str, relative_path: &str) -> super::SkillChoice {
        super::SkillChoice {
            candidate: super::SkillCandidate {
                name: name.to_owned(),
                description: description.to_owned(),
                relative_path: PathBuf::from(relative_path),
            },
            already_installed: false,
        }
    }

    fn shared_plan_item() -> LinkPlanItem {
        LinkPlanItem {
            owners: vec![
                LinkOwner {
                    skill: SkillName::new("review").unwrap(),
                    agent: AgentKind::Pi,
                },
                LinkOwner {
                    skill: SkillName::new("review").unwrap(),
                    agent: AgentKind::custom("universal").unwrap(),
                },
            ],
            source: SourcePath::new("skills/review").unwrap(),
            target: crate::domain::target::TargetPath::new("targets/review").unwrap(),
            action: PlanAction::AlreadySynced,
        }
    }

    #[test]
    fn removing_one_shared_owner_keeps_target() {
        assert!(!super::should_remove_target_for_agents(
            &shared_plan_item(),
            "review",
            &[AgentKind::Pi],
        ));
    }

    #[test]
    fn removing_all_shared_owners_removes_target_once() {
        assert!(super::should_remove_target_for_agents(
            &shared_plan_item(),
            "review",
            &[
                AgentKind::Pi,
                AgentKind::custom("universal").unwrap(),
            ],
        ));
    }

    #[test]
    fn removing_skill_keeps_target_owned_by_another_skill() {
        let mut item = shared_plan_item();
        item.owners[1].skill = SkillName::new("other").unwrap();

        assert!(!super::should_remove_target_for_skill(&item, "review"));
    }

    #[test]
    fn skill_choice_display_is_compact() {
        let choice = skill_choice(
            "review",
            "Review helper with a long explanation",
            "skills/review",
        );

        assert_eq!(choice.to_string(), "review  skills/review");
        assert!(!choice.to_string().contains("long explanation"));
    }

    #[test]
    fn installed_skill_choice_display_marks_already_installed() {
        let choice = super::SkillChoice {
            candidate: super::SkillCandidate {
                name: "review".to_owned(),
                description: "Review helper".to_owned(),
                relative_path: PathBuf::from("skills/review"),
            },
            already_installed: true,
        };

        assert_eq!(
            choice.to_string(),
            "review  skills/review  (already installed)"
        );
    }

    #[test]
    fn selected_skill_formatter_summarizes_many_choices() {
        let choices = [
            skill_choice("one", "First", "skills/one"),
            skill_choice("two", "Second", "skills/two"),
            skill_choice("three", "Third", "skills/three"),
            skill_choice("four", "Fourth", "skills/four"),
            skill_choice("five", "Fifth", "skills/five"),
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
        let choice = skill_choice(
            "diagnose",
            "Hard bugs and performance regressions",
            "skills/engineering/diagnose",
        );

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

    fn candidate(name: &str, description: &str, relative_path: &str) -> super::SkillCandidate {
        super::SkillCandidate {
            name: name.to_owned(),
            description: description.to_owned(),
            relative_path: PathBuf::from(relative_path),
        }
    }

    fn assert_selected(selection: super::CandidateSelection) -> Vec<super::SkillCandidate> {
        match selection {
            super::CandidateSelection::Selected(candidates) => candidates,
            super::CandidateSelection::AllInstalled => panic!("expected a selection"),
        }
    }

    #[test]
    fn name_option_selects_matching_discovered_skill() {
        let selected = assert_selected(
            select_skill_candidates(
                "owner/repo",
                Some("review"),
                vec![
                    candidate("find-skills", "Find skills", "skills/find-skills"),
                    candidate("review", "Review helper", "skills/review"),
                ],
                &BTreeSet::new(),
            )
            .unwrap(),
        );

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].relative_path, Path::new("skills/review"));
    }

    #[test]
    fn build_skill_choices_marks_existing_dependencies_as_installed() {
        let existing = BTreeSet::from(["tdd".to_owned()]);
        let choices = super::build_skill_choices(
            vec![
                candidate("tdd", "Test driven", "skills/tdd"),
                candidate("grilling", "Grill plans", "skills/grilling"),
                candidate("teach", "Teach concepts", "skills/teach"),
            ],
            &existing,
        );

        let installed = choices
            .iter()
            .filter(|choice| choice.already_installed)
            .map(|choice| choice.candidate.name.clone())
            .collect::<Vec<_>>();
        let selectable = choices
            .iter()
            .filter(|choice| !choice.already_installed)
            .map(|choice| choice.candidate.name.clone())
            .collect::<Vec<_>>();

        assert_eq!(installed, vec!["tdd".to_owned()]);
        assert_eq!(selectable, vec!["grilling".to_owned(), "teach".to_owned()]);
    }

    #[test]
    fn name_option_for_installed_skill_fails_with_guidance() {
        let existing = BTreeSet::from(["grilling".to_owned()]);
        let error = select_skill_candidates(
            "owner/repo",
            Some("grilling"),
            vec![
                candidate("grilling", "Grill plans", "skills/grilling"),
                candidate("teach", "Teach concepts", "skills/teach"),
            ],
            &existing,
        )
        .expect_err("installed skill add must fail");

        let message = error.to_string();
        assert!(message.contains("already installed"), "{message}");
        assert!(message.contains("sksync attach"), "{message}");
        assert!(message.contains("sksync update"), "{message}");
        assert!(message.contains("sksync remove"), "{message}");
    }

    #[test]
    fn name_override_for_installed_skill_fails_with_guidance() {
        let existing = BTreeSet::from(["review".to_owned()]);
        let error = select_skill_candidates(
            "owner/repo",
            Some("review"),
            vec![candidate("grilling", "Grill plans", "skills/review")],
            &existing,
        )
        .expect_err("installed name override add must fail");

        let message = error.to_string();
        assert!(message.contains("already installed"), "{message}");
        assert!(message.contains("sksync attach"), "{message}");
    }

    #[test]
    fn all_candidates_installed_reports_nothing_to_add() {
        let existing = BTreeSet::from(["grilling".to_owned(), "teach".to_owned()]);
        let selection = select_skill_candidates(
            "owner/repo",
            None,
            vec![
                candidate("grilling", "Grill plans", "skills/grilling"),
                candidate("teach", "Teach concepts", "skills/teach"),
            ],
            &existing,
        )
        .expect("all-installed is not an error");

        assert!(matches!(selection, super::CandidateSelection::AllInstalled));
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
                global: BTreeMap::from([("pi".to_owned(), PathBuf::from("~/.pi/agent/skills"))]),
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
                global: BTreeMap::from([("pi".to_owned(), PathBuf::from("~/.pi/agent/skills"))]),
                project: BTreeMap::from([("pi".to_owned(), PathBuf::from(".pi/skills"))]),
            },
            Scope::User,
        );

        assert_eq!(mappings["pi"].scope, Scope::User);
        assert_eq!(mappings["pi"].target_dir, Path::new("~/.pi/agent/skills"));
    }

    #[test]
    fn plan_help_is_available() {
        Cli::command()
            .try_get_matches_from(["sksync", "plan", "--help"])
            .expect_err("--help should short-circuit as a clap display error");
    }

    #[test]
    fn force_flags_parse_for_direct_link_commands() {
        Cli::try_parse_from([
            "sksync",
            "add",
            "owner/repo/skills/review",
            "--agent",
            "pi",
            "--force",
        ])
        .expect("add --force parses");
        Cli::try_parse_from(["sksync", "attach", "review", "--agent", "pi", "--force"])
            .expect("attach --force parses");
        Cli::try_parse_from(["sksync", "install", "--force"]).expect("install --force parses");
    }

    #[test]
    fn add_include_flags_parse() {
        Cli::try_parse_from([
            "sksync",
            "add",
            "ogulcancelik/herdr",
            "--agent",
            "pi",
            "--include",
            "SKILL.md",
            "--include",
            "references",
        ])
        .expect("add --include parses");
    }

    #[test]
    fn add_manifest_only_parses() {
        Cli::try_parse_from([
            "sksync",
            "add",
            "ogulcancelik/herdr",
            "--agent",
            "pi",
            "--manifest-only",
        ])
        .expect("add --manifest-only parses");
    }

    #[test]
    fn add_manifest_only_conflicts_with_include() {
        Cli::try_parse_from([
            "sksync",
            "add",
            "ogulcancelik/herdr",
            "--agent",
            "pi",
            "--manifest-only",
            "--include",
            "SKILL.md",
        ])
        .expect_err("manifest-only conflicts with include");
    }

    #[test]
    fn short_force_flags_parse_for_link_commands() {
        Cli::try_parse_from([
            "sksync",
            "add",
            "owner/repo/skills/review",
            "--agent",
            "pi",
            "-f",
        ])
        .expect("add -f parses");
        Cli::try_parse_from(["sksync", "attach", "review", "--agent", "pi", "-f"])
            .expect("attach -f parses");
        Cli::try_parse_from(["sksync", "install", "-f"]).expect("install -f parses");
        Cli::try_parse_from(["sksync", "apply", "-f"]).expect("apply -f parses");
    }

    #[test]
    fn force_flags_parse_for_bundle_link_commands() {
        Cli::try_parse_from([
            "sksync",
            "bundle",
            "add",
            "./bundles/review-workflow",
            "--agent",
            "pi",
            "--force",
        ])
        .expect("bundle add --force parses");
        Cli::try_parse_from(["sksync", "bundle", "sync", "review-workflow", "--force"])
            .expect("bundle sync --force parses");
    }

    #[test]
    fn short_force_flags_parse_for_bundle_commands() {
        Cli::try_parse_from([
            "sksync",
            "bundle",
            "add",
            "./bundles/review-workflow",
            "--agent",
            "pi",
            "-f",
        ])
        .expect("bundle add -f parses");
        Cli::try_parse_from(["sksync", "bundle", "sync", "review-workflow", "-f"])
            .expect("bundle sync -f parses");
        Cli::try_parse_from([
            "sksync",
            "bundle",
            "export",
            "team-baseline",
            "--output",
            "./bundles/team-baseline",
            "-f",
        ])
        .expect("bundle export -f parses");
    }

    #[test]
    fn short_global_flags_parse_for_global_commands() {
        Cli::try_parse_from(["sksync", "init", "-g"]).expect("init -g parses");
        Cli::try_parse_from([
            "sksync",
            "add",
            "owner/repo/skills/review",
            "--agent",
            "pi",
            "-g",
        ])
        .expect("add -g parses");
        Cli::try_parse_from(["sksync", "attach", "review", "--agent", "pi", "-g"])
            .expect("attach -g parses");
        Cli::try_parse_from(["sksync", "doctor", "-g"]).expect("doctor -g parses");
        Cli::try_parse_from(["sksync", "remove", "review", "-g"]).expect("remove -g parses");
        Cli::try_parse_from(["sksync", "outdated", "-g"]).expect("outdated -g parses");
        Cli::try_parse_from(["sksync", "plan", "-g"]).expect("plan -g parses");
        Cli::try_parse_from(["sksync", "apply", "-g"]).expect("apply -g parses");
        Cli::try_parse_from(["sksync", "install", "-g"]).expect("install -g parses");
        Cli::try_parse_from(["sksync", "update", "-g"]).expect("update -g parses");
        Cli::try_parse_from(["sksync", "check", "-g"]).expect("check -g parses");
        Cli::try_parse_from(["sksync", "list", "-g"]).expect("list -g parses");
        Cli::try_parse_from(["sksync", "import", "./skills", "--agent", "pi", "-g"])
            .expect("import -g parses");
        Cli::try_parse_from([
            "sksync",
            "bundle",
            "add",
            "./bundles/review-workflow",
            "--agent",
            "pi",
            "-g",
        ])
        .expect("bundle add -g parses");
        Cli::try_parse_from(["sksync", "bundle", "remove", "review-workflow", "-g"])
            .expect("bundle remove -g parses");
        Cli::try_parse_from(["sksync", "bundle", "sync", "review-workflow", "-g"])
            .expect("bundle sync -g parses");
        Cli::try_parse_from([
            "sksync",
            "bundle",
            "export",
            "team-baseline",
            "--output",
            "./bundles/team-baseline",
            "-g",
        ])
        .expect("bundle export -g parses");
    }

    #[test]
    fn wizard_aliases_are_registered() {
        Cli::try_parse_from(["sksync", "wizard"]).expect("wizard should parse");
        Cli::try_parse_from(["sksync", "ask"]).expect("ask alias should parse");
        Cli::try_parse_from(["sksync", "tui"]).expect("tui alias should parse");
    }

    fn run_git_in(path: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .output()
            .expect("git command runs");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_remote_repo_with_review_skill(path: &Path) {
        fs::create_dir_all(path.join("skills/review")).unwrap();
        run_git_in(path, &["init"]);
        run_git_in(path, &["config", "user.email", "test@example.com"]);
        run_git_in(path, &["config", "user.name", "Test User"]);
        fs::write(
            path.join("skills/review/SKILL.md"),
            "---\nname: review\ndescription: Review helper\n---\n# review\n",
        )
        .unwrap();
        run_git_in(path, &["add", "."]);
        run_git_in(path, &["commit", "-m", "add review"]);
    }

    #[test]
    fn remote_checker_reports_path_missing_for_absent_subpath() {
        let temp = tempfile::tempdir().expect("temp dir");
        let remote = temp.path().join("remote");
        init_remote_repo_with_review_skill(&remote);

        let status = GitRemoteSourceChecker.check_git_source(&GitInstallSource {
            url: remote.display().to_string(),
            reference: None,
            path: "skills/productivity/caveman".into(),
        });

        assert_eq!(status, RemoteSourceStatus::PathMissing);
    }

    #[test]
    fn remote_checker_reports_available_for_existing_subpath() {
        let temp = tempfile::tempdir().expect("temp dir");
        let remote = temp.path().join("remote");
        init_remote_repo_with_review_skill(&remote);

        let status = GitRemoteSourceChecker.check_git_source(&GitInstallSource {
            url: remote.display().to_string(),
            reference: None,
            path: "skills/review".into(),
        });

        assert_eq!(status, RemoteSourceStatus::Available);
    }

    #[test]
    fn remote_checker_reports_unreachable_for_missing_repo() {
        let temp = tempfile::tempdir().expect("temp dir");
        let missing = temp.path().join("does-not-exist");

        let status = GitRemoteSourceChecker.check_git_source(&GitInstallSource {
            url: missing.display().to_string(),
            reference: None,
            path: "skills/review".into(),
        });

        assert!(matches!(status, RemoteSourceStatus::RepoUnreachable(_)));
    }

    /// A fake checker that records how many times it was asked to probe a source.
    /// Used to prove the no-remote default path never performs remote git operations.
    struct CountingChecker {
        status: RemoteSourceStatus,
        calls: std::cell::RefCell<usize>,
    }

    impl CountingChecker {
        fn new(status: RemoteSourceStatus) -> Self {
            Self {
                status,
                calls: std::cell::RefCell::new(0),
            }
        }
    }

    impl RemoteSourceChecker for CountingChecker {
        fn check_git_source(&self, _source: &GitInstallSource) -> RemoteSourceStatus {
            *self.calls.borrow_mut() += 1;
            self.status.clone()
        }
    }

    fn git_dep_config(skill_name: &str, repo: &Path, subpath: &str) -> ResolvedConfig {
        ResolvedConfig {
            skill_dir: SourcePath::new(".sksync/skills").unwrap(),
            agents: BTreeMap::new(),
            skills: vec![ResolvedSkill {
                name: SkillName::new(skill_name).unwrap(),
                source: SourcePath::new(format!(".sksync/skills/{skill_name}")).unwrap(),
                install_source: Some(InstallSource::Git(GitInstallSource {
                    url: repo.display().to_string(),
                    reference: None,
                    path: subpath.into(),
                })),
                include: None,
                agents: Vec::new(),
            }],
            default_agents: Vec::new(),
        }
    }

    // Regression 1: project remote missing path is reported without a --global suggestion.
    #[test]
    fn doctor_remote_project_missing_path_reports_without_global() {
        let temp = tempfile::tempdir().expect("temp dir");
        let remote = temp.path().join("remote");
        init_remote_repo_with_review_skill(&remote);
        let config = git_dep_config("caveman", &remote, "skills/productivity/caveman");

        let report = collect_remote_source_problems(&config, false, &GitRemoteSourceChecker);

        assert_eq!(report.checked, 1);
        assert_eq!(report.problems.len(), 1);
        let problem = &report.problems[0];
        assert_eq!(problem.skill, "caveman");
        assert_eq!(problem.scope_label(), "project");
        assert_eq!(problem.kind, RemoteSourceProblemKind::PathMissing);
        assert_eq!(problem.suggestion(), "sksync remove caveman");
        assert!(!problem.suggestion().contains("--global"));
    }

    // Regression 2: global remote missing path is reported with a --global suggestion.
    #[test]
    fn doctor_remote_global_missing_path_includes_global() {
        let temp = tempfile::tempdir().expect("temp dir");
        let remote = temp.path().join("remote");
        init_remote_repo_with_review_skill(&remote);
        let config = git_dep_config("caveman", &remote, "skills/productivity/caveman");

        let report = collect_remote_source_problems(&config, true, &GitRemoteSourceChecker);

        assert_eq!(report.problems.len(), 1);
        let problem = &report.problems[0];
        assert_eq!(problem.scope_label(), "global");
        assert_eq!(problem.suggestion(), "sksync remove caveman --global");
    }

    // Regression 3: an existing remote path passes the remote check.
    #[test]
    fn doctor_remote_existing_path_passes() {
        let temp = tempfile::tempdir().expect("temp dir");
        let remote = temp.path().join("remote");
        init_remote_repo_with_review_skill(&remote);
        let config = git_dep_config("review", &remote, "skills/review");

        let report = collect_remote_source_problems(&config, false, &GitRemoteSourceChecker);

        assert_eq!(report.checked, 1);
        assert!(report.problems.is_empty());
    }

    // Regression 4: without --remote, doctor performs no remote git operations (no clone).
    #[test]
    fn doctor_default_run_is_local_only_and_never_probes() {
        let config = git_dep_config("caveman", Path::new("/unused"), "skills/missing");
        let config_result: anyhow::Result<ResolvedConfig> = Ok(config);
        let checker = CountingChecker::new(RemoteSourceStatus::PathMissing);

        let problems = doctor_remote_problems(&config_result, false, false, &checker);

        assert!(problems.is_empty());
        assert_eq!(*checker.calls.borrow(), 0);
    }

    // Regression 4 (counterpart): with --remote the gate invokes the checker.
    #[test]
    fn doctor_remote_flag_invokes_checker() {
        let config = git_dep_config("caveman", Path::new("/unused"), "skills/missing");
        let config_result: anyhow::Result<ResolvedConfig> = Ok(config);
        let checker = CountingChecker::new(RemoteSourceStatus::PathMissing);

        let problems = doctor_remote_problems(&config_result, true, false, &checker);

        assert_eq!(problems.len(), 1);
        assert_eq!(*checker.calls.borrow(), 1);
    }

    // A failed config load short-circuits the remote gate without probing.
    #[test]
    fn doctor_remote_skips_when_config_failed_to_load() {
        let config_result: anyhow::Result<ResolvedConfig> = Err(anyhow::anyhow!("no config"));
        let checker = CountingChecker::new(RemoteSourceStatus::PathMissing);

        let problems = doctor_remote_problems(&config_result, true, false, &checker);

        assert!(problems.is_empty());
        assert_eq!(*checker.calls.borrow(), 0);
    }

    // Regression 5: a local dependency source is skipped, never treated as a remote error.
    #[test]
    fn doctor_remote_local_source_is_skipped() {
        let config = ResolvedConfig {
            skill_dir: SourcePath::new(".sksync/skills").unwrap(),
            agents: BTreeMap::new(),
            skills: vec![ResolvedSkill {
                name: SkillName::new("local-helper").unwrap(),
                source: SourcePath::new(".sksync/skills/local-helper").unwrap(),
                install_source: Some(InstallSource::Local(PathBuf::from("./vendor/local-helper"))),
                include: None,
                agents: Vec::new(),
            }],
            default_agents: Vec::new(),
        };

        // Real checker is wired, but a local source must never trigger a clone.
        let report = collect_remote_source_problems(&config, false, &GitRemoteSourceChecker);

        assert!(report.problems.is_empty());
        assert_eq!(report.checked, 0);
        assert_eq!(report.skipped_local, 1);
    }
}
