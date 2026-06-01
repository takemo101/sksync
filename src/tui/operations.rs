use std::path::Path;

use anyhow::Result;

use super::{confirm_and_run, prompt_config_scope, prompt_confirm, run_sksync};

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
