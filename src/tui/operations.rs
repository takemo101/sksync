use std::path::Path;

use anyhow::{Context, Result};
use inquire::Select;

use super::commands::{outdated_args, update_skill_args};
use super::config::load_config_for_scope;
use super::{
    confirm_and_run, prompt_config_scope, prompt_confirm, prompt_skills_from_config, run_sksync,
};

pub(super) fn run_update(project_root: &Path) -> Result<()> {
    let scope = prompt_config_scope("Which config should be updated?")?;
    let mode = Select::new(
        "Which skills should be updated?",
        vec!["All skills", "Selected skills"],
    )
    .with_help_message("Selected updates require an existing lockfile baseline")
    .prompt()
    .context("failed to read update mode")?;
    let skills = if mode == "All skills" {
        Vec::new()
    } else {
        let config = load_config_for_scope(project_root, scope)?;
        prompt_skills_from_config(&config, "Select skill(s) to update")?
    };

    confirm_and_run(
        project_root,
        "Update these skills? Local edits to managed skill bodies will be replaced.",
        update_skill_args(&skills, scope),
    )
}

pub(super) fn run_outdated(project_root: &Path) -> Result<()> {
    let scope = prompt_config_scope("Which config should be inspected for outdated skills?")?;
    run_sksync(project_root, &outdated_args(scope))
}

pub(super) fn run_status(project_root: &Path) -> Result<()> {
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

pub(super) fn run_apply(project_root: &Path) -> Result<()> {
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
