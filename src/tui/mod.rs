use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Command;

use anyhow::{bail, Context, Result};

pub fn run(project_root: PathBuf) -> Result<()> {
    println!("sksync prompt TUI");
    println!("Project: {}", project_root.display());

    loop {
        println!();
        println!("何をしますか?");
        println!("  1) skill を追加する");
        println!("  2) skill を削除する");
        println!("  3) 特定 agent から skill を外す");
        println!("  4) 状態を確認する");
        println!("  5) apply する");
        println!("  q) 終了する");

        match prompt("選択")?.trim() {
            "1" => run_add_flow(&project_root)?,
            "2" => run_remove_flow(&project_root)?,
            "3" => run_remove_agent_flow(&project_root)?,
            "4" => run_status_flow(&project_root)?,
            "5" => run_apply_flow(&project_root)?,
            "q" | "Q" | "quit" | "exit" => return Ok(()),
            value => println!("Unknown selection: {value}"),
        }
    }
}

fn run_add_flow(project_root: &PathBuf) -> Result<()> {
    let source = prompt_required("skill source")?;
    let name = prompt("name override (optional)")?;
    let agents = prompt_agents()?;
    let global = prompt_yes_no("global config に追加しますか?", false)?;

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

    println!("予定: sksync {}", args.join(" "));
    if prompt_yes_no("実行しますか?", false)? {
        run_sksync(project_root, &args)?;
    }
    Ok(())
}

fn run_remove_flow(project_root: &PathBuf) -> Result<()> {
    let skill = prompt_required("skill name")?;
    let global = prompt_yes_no("global config から削除しますか?", false)?;
    let keep_files = prompt_yes_no("skill 本体を残しますか?", false)?;
    let config_only = prompt_yes_no("config / lockfile だけ変更しますか?", false)?;

    let mut args = vec!["remove".to_owned(), skill];
    if global {
        args.push("--global".to_owned());
    }
    if keep_files {
        args.push("--keep-files".to_owned());
    }
    if config_only {
        args.push("--config-only".to_owned());
    }

    println!("予定: sksync {}", args.join(" "));
    if prompt_yes_no("削除を実行しますか?", false)? {
        run_sksync(project_root, &args)?;
    }
    Ok(())
}

fn run_remove_agent_flow(project_root: &PathBuf) -> Result<()> {
    let skill = prompt_required("skill name")?;
    let agents = prompt_agents()?;
    let global = prompt_yes_no("global config を対象にしますか?", false)?;

    let mut args = vec!["remove".to_owned(), skill];
    for agent in agents {
        args.push("--agent".to_owned());
        args.push(agent);
    }
    if global {
        args.push("--global".to_owned());
    }

    println!("予定: sksync {}", args.join(" "));
    if prompt_yes_no("指定 agent から外しますか?", false)? {
        run_sksync(project_root, &args)?;
    }
    Ok(())
}

fn run_status_flow(project_root: &PathBuf) -> Result<()> {
    let global = prompt_yes_no("global config を対象にしますか?", false)?;
    let check = prompt_yes_no("check も実行しますか?", true)?;

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

fn run_apply_flow(project_root: &PathBuf) -> Result<()> {
    let global = prompt_yes_no("global config を対象にしますか?", false)?;
    let force = prompt_yes_no("safe な managed link の置き換えを許可しますか?", false)?;

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

    println!("予定: sksync {}", apply_args.join(" "));
    if prompt_yes_no("apply を実行しますか?", false)? {
        run_sksync(project_root, &apply_args)?;
    }
    Ok(())
}

fn prompt_agents() -> Result<Vec<String>> {
    let input = prompt_required("agents (comma separated, e.g. pi,claude-code)")?;
    let agents = input
        .split(',')
        .map(str::trim)
        .filter(|agent| !agent.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if agents.is_empty() {
        bail!("at least one agent is required");
    }
    Ok(agents)
}

fn prompt_required(label: &str) -> Result<String> {
    let value = prompt(label)?;
    if value.trim().is_empty() {
        bail!("{label} is required");
    }
    Ok(value.trim().to_owned())
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}: ");
    io::stdout().flush().context("failed to flush stdout")?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read stdin")?;
    Ok(input.trim_end().to_owned())
}

fn prompt_yes_no(question: &str, default: bool) -> Result<bool> {
    let suffix = if default { "Y/n" } else { "y/N" };
    let answer = prompt(&format!("{question} ({suffix})"))?;
    match answer.trim().to_ascii_lowercase().as_str() {
        "" => Ok(default),
        "y" | "yes" => Ok(true),
        "n" | "no" => Ok(false),
        value => bail!("expected yes/no answer, got {value:?}"),
    }
}

fn run_sksync(project_root: &PathBuf, args: &[String]) -> Result<()> {
    let exe = std::env::current_exe().context("failed to determine current executable")?;
    let status = Command::new(exe)
        .args(args)
        .current_dir(project_root)
        .status()
        .with_context(|| format!("failed to run sksync {}", args.join(" ")))?;
    if !status.success() {
        bail!("sksync {} failed with {status}", args.join(" "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::prompt_yes_no;

    #[test]
    fn prompt_tui_module_is_available() {
        let run_fn: fn(std::path::PathBuf) -> anyhow::Result<()> = super::run;
        let _ = run_fn;
    }

    #[test]
    fn yes_no_defaults_are_documented_by_type() {
        let fn_ptr: fn(&str, bool) -> anyhow::Result<bool> = prompt_yes_no;
        let _ = fn_ptr;
    }
}
