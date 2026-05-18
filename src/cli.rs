use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};

use crate::application::plan::build_link_plan;
use crate::application::ports::ConfigStore;
use crate::infrastructure::builtin_agents::TargetPathResolver;
use crate::infrastructure::fs::FileSystemLinkStore;
use crate::infrastructure::json::FileConfigStore;

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
        Command::Apply(args) => {
            let _force = args.force;
            not_implemented("apply")
        }
        Command::Check => not_implemented("check"),
        Command::List => not_implemented("list"),
        Command::Tui => not_implemented("tui"),
    }
}

fn run_plan(_args: PlanArgs) -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    let config_store = FileConfigStore::new(current_dir.join("sksync.config.json"));
    let config = config_store.load()?;
    let fs_store = FileSystemLinkStore;
    let target_resolver = TargetPathResolver::new(&current_dir, home_dir);
    let plan = build_link_plan(&config, &fs_store, &fs_store, &target_resolver)?;

    if plan.is_empty() {
        println!("No actions planned.");
    } else {
        for line in plan.display_lines() {
            println!("{line}");
        }
    }

    Ok(())
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
