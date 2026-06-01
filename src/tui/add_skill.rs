use std::fmt;
use std::path::Path;

use anyhow::{bail, Context, Result};
use inquire::{Select, Text};

use super::commands::add_skill_args;
use super::config::load_optional_config_for_scope;
use super::default_agents::default_agents_from_config;
use super::{confirm_and_run, prompt_agents, prompt_config_scope, prompt_confirm, prompt_required};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PackageFilterChoice {
    FullPackage,
    ManifestOnly,
    Custom(Vec<String>),
}

impl fmt::Display for PackageFilterChoice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::FullPackage => "Full package",
            Self::ManifestOnly => "Manifest only (SKILL.md)",
            Self::Custom(_) => "Custom include patterns",
        };
        formatter.write_str(label)
    }
}

pub(super) fn run(project_root: &Path) -> Result<()> {
    let source = prompt_required("Skill source")?;
    let name = Text::new("Name override")
        .with_help_message("Optional; leave blank to infer from source")
        .prompt()
        .context("failed to read name override")?;
    let package_filter = prompt_package_filter_choice()?;
    let scope = prompt_config_scope("Where should this dependency be added?")?;
    let config = load_optional_config_for_scope(project_root, scope)?;
    let default_agents = default_agents_from_config(config.as_ref());
    let agents = prompt_agents(scope, config.as_ref(), &default_agents)?;
    let args = add_skill_args(&source, name.trim(), &agents, scope, &package_filter);

    confirm_and_run(project_root, "Run this command?", args)
}

fn prompt_package_filter_choice() -> Result<PackageFilterChoice> {
    let choice = Select::new(
        "Which package files should be installed?",
        vec![
            PackageFilterChoice::FullPackage,
            PackageFilterChoice::ManifestOnly,
            PackageFilterChoice::Custom(Vec::new()),
        ],
    )
    .with_help_message("Full package keeps current behavior; manifest-only copies only SKILL.md.")
    .prompt()
    .context("failed to read package filter choice")?;

    match choice {
        PackageFilterChoice::Custom(_) => {
            let patterns = prompt_include_patterns()?;
            Ok(PackageFilterChoice::Custom(patterns))
        }
        other => Ok(other),
    }
}

fn prompt_include_patterns() -> Result<Vec<String>> {
    let mut patterns = Vec::new();
    loop {
        let value = Text::new("Include pattern")
            .with_help_message(
                "One pattern relative to the skill package root, e.g. SKILL.md or references",
            )
            .prompt()
            .context("failed to read include pattern")?;
        patterns.push(validate_include_pattern(&value)?);

        if !prompt_confirm("Add another include pattern?", false)? {
            break;
        }
    }
    crate::domain::package_filter::PackageFilter::new(patterns.clone())
        .context("invalid include patterns")?;
    Ok(patterns)
}

fn validate_include_pattern(value: &str) -> Result<String> {
    let pattern = value.trim();
    if pattern.contains(',') || pattern.contains('\n') || pattern.contains('\r') {
        bail!("enter one include pattern at a time")
    }
    crate::domain::package_filter::PackageFilter::new(vec![pattern.to_owned()])
        .context("invalid include pattern")?;
    Ok(pattern.to_owned())
}

#[cfg(test)]
mod tests {
    use super::validate_include_pattern;

    #[test]
    fn include_pattern_prompt_accepts_one_pattern_at_a_time() {
        assert_eq!(validate_include_pattern(" SKILL.md ").unwrap(), "SKILL.md");
        assert!(validate_include_pattern("SKILL.md, references").is_err());
        assert!(validate_include_pattern("SKILL.md\nreferences").is_err());
    }
}
