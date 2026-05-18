use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};

use crate::application::apply::{apply_link_plan, ApplyOptions};
use crate::application::config::ResolvedConfig;
use crate::application::plan::build_link_plan;
use crate::application::ports::ConfigStore;
use crate::domain::link_plan::LinkPlan;
use crate::domain::lockfile::{LinkType, LockedFile, LockedSkill, LockedTarget, Lockfile};
use crate::infrastructure::builtin_agents::TargetPathResolver;
use crate::infrastructure::fs::FileSystemLinkStore;
use crate::infrastructure::hash::hash_directory;
use crate::infrastructure::json::{FileConfigStore, FileLockfileStore};

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
    /// Show the synchronization plan without changing the filesystem.
    Plan(PlanArgs),
    /// Apply the synchronization plan to the filesystem.
    Apply(ApplyArgs),
    /// Check config, lockfile, hashes, and symlink health.
    Check,
    /// List managed skills and agent link status.
    List,
    /// Launch the interactive terminal UI.
    Tui,
}

#[derive(Debug, Args)]
struct PlanArgs {
    /// Explicitly run in dry-run mode.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Args)]
struct ApplyArgs {
    /// Allow replacing existing sksync-managed links when it is safe to do so.
    #[arg(long)]
    force: bool,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    dispatch(cli.command)
}

fn dispatch(command: Command) -> Result<()> {
    match command {
        Command::Init => not_implemented("init"),
        Command::Plan(args) => run_plan(args),
        Command::Apply(args) => run_apply(args),
        Command::Check => not_implemented("check"),
        Command::List => not_implemented("list"),
        Command::Tui => not_implemented("tui"),
    }
}

fn run_plan(_args: PlanArgs) -> Result<()> {
    let (_config, plan, _current_dir) = load_plan()?;
    print_plan(&plan);
    Ok(())
}

fn run_apply(args: ApplyArgs) -> Result<()> {
    let (config, plan, current_dir) = load_plan()?;
    let lockfile = build_lockfile_from_plan(&config, &plan, &current_dir)?;
    let fs_store = FileSystemLinkStore;
    let lockfile_store = FileLockfileStore::new(current_dir.join("sksync-lock.json"));

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

fn load_plan() -> Result<(ResolvedConfig, LinkPlan, PathBuf)> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    let config_store = FileConfigStore::new(current_dir.join("sksync.config.json"));
    let config = config_store.load()?;
    let fs_store = FileSystemLinkStore;
    let target_resolver = TargetPathResolver::new(&current_dir, home_dir);
    let plan = build_link_plan(&config, &fs_store, &fs_store, &target_resolver)?;

    Ok((config, plan, current_dir))
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

fn not_implemented(command: &str) -> Result<()> {
    bail!("sksync {command} is not implemented yet")
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

        assert_eq!(names, ["init", "plan", "apply", "check", "list", "tui"]);
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
